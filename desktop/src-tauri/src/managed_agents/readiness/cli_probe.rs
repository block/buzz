use std::path::Path;
use std::time::{Duration, Instant};

use crate::managed_agents::runtime::build_augmented_path;

/// Build the augmented PATH for CLI probes and other native child processes
/// (auth commands, `buzz-acp models` discovery), including nvm's default
/// Node.js bin directory so `#!/usr/bin/env node` shims (e.g. codex-acp)
/// resolve.
pub(crate) fn augmented_path() -> Option<String> {
    let home = dirs::home_dir();
    let nvm_bin = home
        .as_deref()
        .and_then(crate::managed_agents::find_nvm_default_bin);
    build_augmented_path(
        home,
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
        crate::managed_agents::login_shell_path(),
        nvm_bin,
    )
}

/// Outcome of a CLI login-status probe.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProbeOutcome {
    /// The CLI reported a successful login (exit 0).
    LoggedIn,
    /// The CLI exited non-zero without a config-parse signal — treat as
    /// "not authenticated."
    LoggedOut,
    /// The CLI exited non-zero and its stderr contains a config-parse error
    /// (e.g. from `~/.codex/config.toml`). The user needs to fix their
    /// config, not re-run login.
    ConfigInvalid {
        /// A trimmed excerpt of the stderr message to surface in the nudge.
        stderr_excerpt: String,
    },
}

/// Signals emitted to stderr by codex (and related CLI tools) when they
/// fail to parse their config file. We check these to distinguish a
/// config-parse failure from a genuine "not authenticated" exit.
///
/// The real codex error reads:
///   `Error loading configuration: .../.codex/config.toml:... unknown variant ...`
/// So we require BOTH "error loading configuration" AND "unknown variant" to be
/// present, avoiding false positives from unrelated errors that mention only
/// one term.
const CONFIG_PARSE_SIGNALS: &[&str] = &["error loading configuration", "unknown variant"];

/// Run the probe at the resolved absolute path so the GUI-PATH gap is
/// bypassed. Injects the same augmented PATH used for launched agents so
/// script shims with `/usr/bin/env <interpreter>` shebangs can find runtimes
/// such as node/python when the app was launched with a bare GUI PATH.
pub(crate) fn login_probe(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
) -> ProbeOutcome {
    let mut command = std::process::Command::new(binary_path);
    command.args(&probe_args[1..]);
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    crate::util::configure_no_window(&mut command);

    match command.output() {
        Ok(o) if o.status.success() => ProbeOutcome::LoggedIn,
        Ok(o) => classify_probe_output(&o.stderr, false),
        Err(_) => ProbeOutcome::LoggedOut,
    }
}

/// Retry policy for [`login_probe_with_recheck`].
///
/// Guards against the transient-flap failure mode where a CLI auth probe
/// returns a false negative for a fraction of a second (typically while the
/// underlying credential store is refreshing on-demand), and the desktop
/// snapshots that false negative into `BUZZ_ACP_SETUP_PAYLOAD` for the
/// lifetime of the spawned harness process. `buzz-acp` explicitly never
/// re-derives readiness after startup, so a single false negative traps the
/// agent in setup-listener mode.
///
/// The default policy [`RetryPolicy::startup_readiness_default`] runs at
/// most three fast attempts (250 ms apart) plus one authoritative final
/// recheck (500 ms after the last attempt). Worst-case added latency
/// versus the pre-fix single probe: ~1 s when auth is genuinely broken;
/// zero when the first probe already returns `LoggedIn`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryPolicy {
    /// Total probe attempts before the final recheck. Must be ≥ 1.
    pub max_attempts: u32,
    /// Delay between successive probe attempts.
    pub backoff: Duration,
    /// Delay before the authoritative final recheck runs. The recheck runs
    /// only if every prior attempt returned `LoggedOut`; a single `LoggedIn`
    /// or `ConfigInvalid` short-circuits the whole sequence.
    pub final_recheck_delay: Duration,
}

impl RetryPolicy {
    /// Default startup-readiness policy: three attempts, 250 ms backoff,
    /// 500 ms final-recheck delay. Chosen to fit inside the desktop's
    /// spawn latency budget while covering the sub-second flap window
    /// observed in the Fizz Air incident (2026-08-23).
    pub(crate) fn startup_readiness_default() -> Self {
        Self {
            max_attempts: 3,
            backoff: Duration::from_millis(250),
            final_recheck_delay: Duration::from_millis(500),
        }
    }
}

/// Run the login probe with a bounded retry sequence and one authoritative
/// final recheck.
///
/// Semantics:
/// * If any attempt returns `LoggedIn` or `ConfigInvalid`, that outcome is
///   returned immediately — `ConfigInvalid` is not a transient state, so
///   retrying it would only stall the spawn without changing the result.
/// * If every attempt through `policy.max_attempts` returns `LoggedOut`,
///   sleep for `policy.final_recheck_delay` and run one more probe. The
///   recheck's outcome is authoritative — this is the "final recheck"
///   that breaks a stuck negative when auth turned green during the
///   backoff window.
/// * Every attempt's diagnostic (attempt index, elapsed millis, and the
///   first stderr line on non-success) is written to stderr via
///   `eprintln!` so the desktop log preserves the error trail the pre-fix
///   single-shot path silently discarded.
pub(crate) fn login_probe_with_recheck(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
    policy: RetryPolicy,
) -> ProbeOutcome {
    let max_attempts = policy.max_attempts.max(1);
    let binary_display = binary_path.display();
    for attempt in 1..=max_attempts {
        let started = Instant::now();
        let outcome = login_probe(binary_path, probe_args, augmented_path);
        log_probe_attempt(&binary_display, attempt, max_attempts, &outcome, started);
        match outcome {
            ProbeOutcome::LoggedIn | ProbeOutcome::ConfigInvalid { .. } => return outcome,
            ProbeOutcome::LoggedOut if attempt < max_attempts => {
                std::thread::sleep(policy.backoff);
            }
            ProbeOutcome::LoggedOut => {}
        }
    }
    // Every attempt returned LoggedOut. Take the final authoritative recheck.
    std::thread::sleep(policy.final_recheck_delay);
    let started = Instant::now();
    let final_outcome = login_probe(binary_path, probe_args, augmented_path);
    log_probe_attempt(
        &binary_display,
        max_attempts + 1,
        max_attempts + 1,
        &final_outcome,
        started,
    );
    final_outcome
}

/// Emit one line per attempt so operators can reconstruct the retry trail.
/// Never panics; logging failures are ignored on purpose.
fn log_probe_attempt(
    binary_display: &std::path::Display<'_>,
    attempt: u32,
    total: u32,
    outcome: &ProbeOutcome,
    started: Instant,
) {
    let elapsed_ms = started.elapsed().as_millis();
    let label = match outcome {
        ProbeOutcome::LoggedIn => "logged_in",
        ProbeOutcome::LoggedOut => "logged_out",
        ProbeOutcome::ConfigInvalid { .. } => "config_invalid",
    };
    let excerpt = match outcome {
        ProbeOutcome::ConfigInvalid { stderr_excerpt } => {
            format!(" stderr={:?}", excerpt_first_line(stderr_excerpt))
        }
        _ => String::new(),
    };
    eprintln!(
        "buzz-desktop: cli_probe attempt {attempt}/{total} outcome={label} elapsed_ms={elapsed_ms} binary={binary_display}{excerpt}"
    );
}

fn excerpt_first_line(s: &str) -> &str {
    s.trim().lines().next().unwrap_or("")
}

/// Classify collected probe output into a `ProbeOutcome`.
///
/// Shared between `login_probe` (which has the full `Output`) and the
/// process-level timeout path in `probe_auth_status` (which drains stderr
/// on a background thread and collects it separately).
pub(crate) fn classify_probe_output(stderr_bytes: &[u8], exit_success: bool) -> ProbeOutcome {
    if exit_success {
        return ProbeOutcome::LoggedIn;
    }
    let stderr = String::from_utf8_lossy(stderr_bytes);
    let stderr_lower = stderr.to_lowercase();
    if CONFIG_PARSE_SIGNALS
        .iter()
        .all(|sig| stderr_lower.contains(sig))
    {
        let excerpt = stderr.trim().lines().next().unwrap_or("").to_string();
        ProbeOutcome::ConfigInvalid {
            stderr_excerpt: excerpt,
        }
    } else {
        ProbeOutcome::LoggedOut
    }
}

#[cfg(test)]
mod tests {
    use super::{ProbeOutcome, CONFIG_PARSE_SIGNALS};

    #[cfg(unix)]
    #[test]
    fn login_probe_uses_augmented_path_for_env_shebang_interpreter() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let temp = tempfile::tempdir().expect("temp dir");
        let script_dir = temp.path().join("script-bin");
        let interpreter_dir = temp.path().join("interpreter-bin");
        let empty_path_dir = temp.path().join("empty-bin");
        fs::create_dir_all(&script_dir).expect("script dir");
        fs::create_dir_all(&interpreter_dir).expect("interpreter dir");
        fs::create_dir_all(&empty_path_dir).expect("empty path dir");

        let interpreter_path = interpreter_dir.join("node");
        let marker_path = temp.path().join("fake-node-ran");
        fs::write(
            &interpreter_path,
            format!(
                "#!/bin/sh\nprintf 'fake node ran\\n' > '{}' || exit 1\nexit 0\n",
                marker_path.display()
            ),
        )
        .expect("write interpreter");
        fs::set_permissions(&interpreter_path, fs::Permissions::from_mode(0o755))
            .expect("chmod interpreter");

        let script_path = script_dir.join("fake-codex");
        fs::write(&script_path, "#!/usr/bin/env node\n").expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let scrubbed_path = std::env::join_paths([empty_path_dir.as_path()])
            .expect("join scrubbed PATH")
            .to_string_lossy()
            .into_owned();
        let without_augmented_path = Command::new(&script_path)
            .args(["login", "status"])
            .env("PATH", &scrubbed_path)
            .output()
            .expect("run script with scrubbed PATH");
        assert!(
            !without_augmented_path.status.success(),
            "with a scrubbed PATH, /usr/bin/env should not find node"
        );

        let augmented_path =
            std::env::join_paths([interpreter_dir.as_path()]).expect("join augmented PATH");
        let augmented_path = augmented_path.to_string_lossy().into_owned();
        assert_eq!(
            super::login_probe(
                &script_path,
                &["fake-codex", "login", "status"],
                Some(&augmented_path),
            ),
            ProbeOutcome::LoggedIn,
            "the injected augmented PATH should allow /usr/bin/env to find the interpreter"
        );
        assert!(
            marker_path.exists(),
            "the fake node from the injected PATH should have run"
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_config_invalid_on_stderr_signal() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");

        // Script that exits 1 and writes a codex-style config-parse error to stderr.
        let script_path = bin_dir.join("fake-codex-bad-config");
        fs::write(
            &script_path,
            "#!/bin/sh\necho 'Error loading configuration: /home/user/.codex/config.toml: unknown variant `ultra`, expected one of none/minimal/low/medium/high/xhigh' >&2\nexit 1\n",
        )
        .expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let outcome = super::login_probe(
            &script_path,
            &["fake-codex-bad-config", "login", "status"],
            None,
        );
        assert!(
            matches!(outcome, ProbeOutcome::ConfigInvalid { .. }),
            "stderr with 'unknown variant' should produce ConfigInvalid; got {:?}",
            outcome
        );
        if let ProbeOutcome::ConfigInvalid { stderr_excerpt } = outcome {
            assert!(
                stderr_excerpt.contains("unknown variant")
                    || stderr_excerpt.contains("Error loading"),
                "stderr_excerpt should contain the parse error: {stderr_excerpt}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_logged_out_on_nonzero_without_config_signal() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");

        // Script that exits 1 with a generic "not logged in" message.
        let script_path = bin_dir.join("fake-codex-logged-out");
        fs::write(
            &script_path,
            "#!/bin/sh\necho 'not authenticated' >&2\nexit 1\n",
        )
        .expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let outcome = super::login_probe(
            &script_path,
            &["fake-codex-logged-out", "login", "status"],
            None,
        );
        assert_eq!(
            outcome,
            ProbeOutcome::LoggedOut,
            "non-config stderr should produce LoggedOut"
        );
    }

    /// Verify that every string in CONFIG_PARSE_SIGNALS is lowercased so the
    /// case-insensitive match works correctly.
    #[test]
    fn config_parse_signals_are_lowercase() {
        for sig in CONFIG_PARSE_SIGNALS {
            assert_eq!(
                *sig,
                sig.to_lowercase(),
                "CONFIG_PARSE_SIGNAL must be lowercase for case-insensitive matching: {sig}"
            );
        }
    }

    // ── Bounded-retry + final-recheck regression tests ────────────────────
    //
    // Guards the failure mode observed on Fizz Air 2026-08-23: a single
    // sub-second false-negative from `claude auth status` at spawn time
    // trapped the harness in setup-listener mode for the lifetime of the
    // process because the pre-fix single-shot probe discarded the flap
    // and buzz-acp never re-derives readiness.

    #[cfg(unix)]
    fn write_counter_script(
        dir: &std::path::Path,
        filename: &str,
        body: &str,
    ) -> std::path::PathBuf {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(filename);
        fs::write(&path, body).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod script");
        path
    }

    /// A single transient false-negative in the middle of otherwise-green
    /// probes must NOT produce `LoggedOut` — the bounded retry recovers
    /// and returns `LoggedIn`.
    #[cfg(unix)]
    #[test]
    fn login_probe_with_recheck_recovers_from_transient_flap() {
        use std::fs;
        let temp = tempfile::tempdir().expect("temp dir");
        let counter_path = temp.path().join("attempts");
        fs::write(&counter_path, "").expect("init counter");
        // Script increments the counter and exits 1 on attempt 1, exit 0 thereafter.
        let script_body = format!(
            "#!/bin/sh\nprintf 'x' >> '{}'\nn=$(wc -c < '{}' | tr -d ' ')\nif [ \"$n\" -eq 1 ]; then\n  echo 'transient flap' >&2\n  exit 1\nfi\nexit 0\n",
            counter_path.display(),
            counter_path.display()
        );
        let script = write_counter_script(temp.path(), "fake-claude-flap", &script_body);
        let policy = super::RetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::from_millis(1),
            final_recheck_delay: std::time::Duration::from_millis(1),
        };
        let outcome = super::login_probe_with_recheck(
            &script,
            &["fake-claude-flap", "auth", "status"],
            None,
            policy,
        );
        assert_eq!(
            outcome,
            ProbeOutcome::LoggedIn,
            "one-attempt flap must recover to LoggedIn under bounded retry"
        );
        // Exactly 2 attempts: one flap, then the retry that succeeds.
        let attempts_seen = std::fs::read_to_string(&counter_path)
            .expect("read attempts counter")
            .len();
        assert_eq!(
            attempts_seen, 2,
            "expected 2 probe runs (flap + retry-that-succeeds); got {attempts_seen}"
        );
    }

    /// If every retry attempt fails AND the final recheck also fails, the
    /// authoritative outcome is `LoggedOut`. This is the genuine-not-authed
    /// case and must NOT be masked by the retry loop.
    #[cfg(unix)]
    #[test]
    fn login_probe_with_recheck_returns_logged_out_when_all_attempts_fail() {
        use std::fs;
        let temp = tempfile::tempdir().expect("temp dir");
        let counter_path = temp.path().join("attempts");
        fs::write(&counter_path, "").expect("init counter");
        let script_body = format!(
            "#!/bin/sh\nprintf 'x' >> '{}'\necho 'not authenticated' >&2\nexit 1\n",
            counter_path.display()
        );
        let script = write_counter_script(temp.path(), "fake-claude-always-out", &script_body);
        let policy = super::RetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::from_millis(1),
            final_recheck_delay: std::time::Duration::from_millis(1),
        };
        let outcome = super::login_probe_with_recheck(
            &script,
            &["fake-claude-always-out", "auth", "status"],
            None,
            policy,
        );
        assert_eq!(
            outcome,
            ProbeOutcome::LoggedOut,
            "genuine LoggedOut must survive the retry sequence"
        );
        // max_attempts (3) + one final recheck = 4 total runs.
        let attempts_seen = std::fs::read_to_string(&counter_path)
            .expect("read attempts counter")
            .len();
        assert_eq!(
            attempts_seen, 4,
            "expected 3 retries + 1 final recheck = 4 probe runs; got {attempts_seen}"
        );
    }

    /// A `ConfigInvalid` verdict is not a transient condition — retrying it
    /// would only stall the spawn. First occurrence must short-circuit the
    /// whole retry sequence.
    #[cfg(unix)]
    #[test]
    fn login_probe_with_recheck_short_circuits_on_config_invalid() {
        use std::fs;
        let temp = tempfile::tempdir().expect("temp dir");
        let counter_path = temp.path().join("attempts");
        fs::write(&counter_path, "").expect("init counter");
        let script_body = format!(
            "#!/bin/sh\nprintf 'x' >> '{}'\necho 'Error loading configuration: /home/user/.codex/config.toml: unknown variant `ultra`' >&2\nexit 1\n",
            counter_path.display()
        );
        let script = write_counter_script(temp.path(), "fake-codex-bad-config", &script_body);
        let policy = super::RetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::from_millis(1),
            final_recheck_delay: std::time::Duration::from_millis(1),
        };
        let outcome = super::login_probe_with_recheck(
            &script,
            &["fake-codex-bad-config", "login", "status"],
            None,
            policy,
        );
        assert!(
            matches!(outcome, ProbeOutcome::ConfigInvalid { .. }),
            "ConfigInvalid must short-circuit retry; got {outcome:?}"
        );
        let attempts_seen = std::fs::read_to_string(&counter_path)
            .expect("read attempts counter")
            .len();
        assert_eq!(
            attempts_seen, 1,
            "ConfigInvalid should short-circuit after 1 attempt; got {attempts_seen}"
        );
    }

    /// The default startup-readiness policy is the load-bearing knob wired
    /// into `cli_login::requirements`. Lock its shape so a well-meaning
    /// tweak to the constants cannot silently expand the spawn budget
    /// past what the desktop tolerates.
    #[test]
    fn startup_readiness_default_policy_stays_bounded() {
        let p = super::RetryPolicy::startup_readiness_default();
        assert_eq!(p.max_attempts, 3, "default max_attempts drifted");
        assert_eq!(
            p.backoff.as_millis(),
            250,
            "default backoff drifted; a longer backoff extends the spawn budget"
        );
        assert_eq!(
            p.final_recheck_delay.as_millis(),
            500,
            "default final_recheck_delay drifted; keep total worst-case ≤ 1 s"
        );
        // Worst-case total added latency = (max_attempts - 1) * backoff + final_recheck_delay
        let worst_case_ms = (p.max_attempts as u128 - 1) * p.backoff.as_millis()
            + p.final_recheck_delay.as_millis();
        assert!(
            worst_case_ms <= 1_000,
            "startup-readiness retry policy worst case must stay ≤ 1000 ms; got {worst_case_ms}"
        );
    }
}
