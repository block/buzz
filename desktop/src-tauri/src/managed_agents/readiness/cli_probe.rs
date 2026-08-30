use std::path::Path;
use std::time::Duration;

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

/// Delays between successive probe attempts for [`login_probe_with_recheck`].
/// The initial attempt has no leading delay; each entry is the delay before
/// the *next* attempt. `PROBE_ATTEMPT_DELAYS.len() + 1` = total attempts.
///
/// Rationale: `buzz-acp/setup_mode.rs` snapshots the readiness payload at
/// spawn time and explicitly never re-derives it, so a single transient
/// non-zero probe on startup traps the agent in setup-listener mode for
/// the child process's lifetime. The observed transient window in the
/// Fizz Air incident (2026-08-23) was sub-second — the CLI probe read
/// `loggedIn=false` for an instant while the credential store was
/// refreshing, then returned to green seconds later. Three quick
/// attempts (250 ms + 500 ms backoff) catch a sub-second flap; the
/// authoritative final recheck (1000 ms later) catches the "green again
/// seconds later" case. On the truly logged-out path this schedule
/// contributes ~1.75 s of added *sleep*; each per-attempt
/// `Command::output()` remains wall-clock-unbounded, inheriting the
/// pre-fix single-shot behavior (adding a per-attempt subprocess timeout
/// is deliberately deferred as a separable follow-up so this fix keeps
/// its blast radius to the retry loop). First-attempt success adds zero
/// latency and never invokes the sleeper.
const PROBE_ATTEMPT_DELAYS: &[Duration] = &[
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1000),
];

/// Run a single-shot login probe at the resolved absolute path so the
/// GUI-PATH gap is bypassed. Injects the same augmented PATH used for
/// launched agents so script shims with `/usr/bin/env <interpreter>`
/// shebangs can find runtimes such as node/python when the app was
/// launched with a bare GUI PATH.
///
/// Used by the status-polling / config-diff readiness path where
/// spending ~1.75 s of retry sleeps per logged-out row per poll would
/// pin transition/store locks and starve the UI. Callers that need to
/// protect a snapshot against a transient false negative — currently
/// only the spawn-time readiness check that seeds
/// `BUZZ_ACP_SETUP_PAYLOAD` — should use [`login_probe_with_recheck`]
/// instead.
pub(crate) fn login_probe_single_shot(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
) -> ProbeOutcome {
    match run_probe(binary_path, probe_args, augmented_path) {
        AttemptOutcome::Definitive(outcome) => outcome,
        AttemptOutcome::Transient(_) => ProbeOutcome::LoggedOut,
    }
}

/// Legacy alias — the status-poll caller in `cli_login::requirements` uses
/// this name, and keeping it lets `cli_login.rs` stay byte-identical to
/// `origin/main`. New callers should prefer [`login_probe_single_shot`]
/// (for status polling) or [`login_probe_with_recheck`] (for spawn).
pub(crate) use login_probe_single_shot as login_probe;

/// Run the login probe with a bounded retry sequence and one
/// authoritative final recheck. Preserves the last transient diagnostic
/// via `tracing::warn!` when every attempt fails so the reason for the
/// eventual `LoggedOut` verdict is recoverable from the desktop log
/// instead of being silently discarded.
///
/// Semantics:
/// * `LoggedIn` short-circuits — returned on the first successful attempt.
/// * `ConfigInvalid` short-circuits — not a transient state; retrying
///   would only stall the spawn without changing the outcome.
/// * A transient failure (non-zero exit without the config-parse signal,
///   or an exec error such as ENOENT) triggers the next attempt after
///   the corresponding [`PROBE_ATTEMPT_DELAYS`] delay.
/// * If every attempt is transient, the last diagnostic is logged and
///   `LoggedOut` is returned.
pub(crate) fn login_probe_with_recheck(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
) -> ProbeOutcome {
    let (outcome, last_transient) = login_probe_with_recheck_impl(
        binary_path,
        probe_args,
        augmented_path,
        PROBE_ATTEMPT_DELAYS,
        std::thread::sleep,
    );
    if let (ProbeOutcome::LoggedOut, Some(diag)) = (&outcome, last_transient) {
        tracing::warn!(
            target: "buzz_desktop::cli_probe",
            "startup readiness probe {} declared logged-out after {} attempt(s): {}",
            binary_path.display(),
            PROBE_ATTEMPT_DELAYS.len() + 1,
            diag.describe(),
        );
    }
    outcome
}

/// Testable core of [`login_probe_with_recheck`]. Extracted so unit tests
/// can inject a no-op sleeper (skipping the real backoffs) and a custom
/// delay schedule (asserting attempt counts), and can assert the last
/// transient diagnostic that fell through to `LoggedOut`. The wrapper
/// [`login_probe_with_recheck`] emits the same diagnostic via
/// `tracing::warn!` so operators can recover it from the desktop log.
pub(super) fn login_probe_with_recheck_impl<S>(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
    delays: &[Duration],
    mut sleep: S,
) -> (ProbeOutcome, Option<TransientDiagnostic>)
where
    S: FnMut(Duration),
{
    let total_attempts = delays.len() + 1;
    let mut last_transient: Option<TransientDiagnostic> = None;

    for attempt in 0..total_attempts {
        if attempt > 0 {
            sleep(delays[attempt - 1]);
        }
        match run_probe(binary_path, probe_args, augmented_path) {
            AttemptOutcome::Definitive(outcome) => return (outcome, None),
            AttemptOutcome::Transient(diag) => {
                last_transient = Some(diag);
            }
        }
    }
    (ProbeOutcome::LoggedOut, last_transient)
}

/// Result of a single probe invocation, distinguishing outcomes that
/// should short-circuit the retry loop from those that should trigger
/// another attempt.
enum AttemptOutcome {
    /// Terminal outcome — do not retry.
    Definitive(ProbeOutcome),
    /// Transient failure — retry if the budget allows. Carries the
    /// diagnostic material so the last failure can be logged if every
    /// attempt is transient.
    Transient(TransientDiagnostic),
}

/// Diagnostic material captured from a transient probe failure. Emitted
/// via `tracing::warn!` by [`login_probe_with_recheck`] when every
/// attempt fails, and returned by [`login_probe_with_recheck_impl`] so
/// unit tests can assert the last-failure-wins invariant that the
/// operator-facing log line depends on.
#[cfg_attr(test, derive(Debug))]
pub(super) enum TransientDiagnostic {
    /// The subprocess exited non-zero without a config-parse signal.
    NonZero {
        exit_code: Option<i32>,
        stderr_excerpt: String,
    },
    /// The subprocess could not be spawned at all.
    Exec { error: String },
}

impl TransientDiagnostic {
    /// One-line human summary suitable for a `tracing::warn!` message.
    pub(super) fn describe(&self) -> String {
        match self {
            TransientDiagnostic::NonZero {
                exit_code,
                stderr_excerpt,
            } => {
                let code = exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "<signaled>".to_string());
                if stderr_excerpt.is_empty() {
                    format!("exit {code}, no stderr")
                } else {
                    format!("exit {code}, stderr: {stderr_excerpt}")
                }
            }
            TransientDiagnostic::Exec { error } => format!("exec error: {error}"),
        }
    }
}

/// Invoke the probe binary once and classify the result. Extracted from
/// [`login_probe`] so the retry loop can distinguish transient failures
/// (which carry retry-eligible diagnostic material) from terminal outcomes
/// (`LoggedIn`, `ConfigInvalid`).
fn run_probe(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
) -> AttemptOutcome {
    let mut command = std::process::Command::new(binary_path);
    command.args(&probe_args[1..]);
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    crate::util::configure_no_window(&mut command);
    // `Stdio::null()` on stdout at the OS level: the probe's stdout
    // bytes never enter this process. Nothing downstream — the retry
    // loop, the diagnostic, the `tracing::warn!` sink — can ever
    // surface anything the CLI wrote to stdout (e.g. the raw JSON of
    // `claude auth status`, which may embed session identifiers).
    // Belt-and-braces: even if a later refactor accidentally reads
    // stdout, the OS handle is already closed.
    command.stdout(std::process::Stdio::null());

    match command.output() {
        Ok(output) if output.status.success() => AttemptOutcome::Definitive(ProbeOutcome::LoggedIn),
        Ok(output) => match classify_probe_output(&output.stderr, false) {
            outcome @ ProbeOutcome::ConfigInvalid { .. } => AttemptOutcome::Definitive(outcome),
            // classify_probe_output cannot return LoggedIn when
            // exit_success=false, but fall through defensively.
            ProbeOutcome::LoggedOut | ProbeOutcome::LoggedIn => {
                AttemptOutcome::Transient(TransientDiagnostic::NonZero {
                    exit_code: output.status.code(),
                    stderr_excerpt: sanitize_stderr_excerpt(&output.stderr),
                })
            }
        },
        Err(err) => AttemptOutcome::Transient(TransientDiagnostic::Exec {
            error: bound_diagnostic_text(&err.to_string()),
        }),
    }
}

/// Total bytes allowed in any operator-facing diagnostic string,
/// **including** the trailing ellipsis on truncation. Chosen tight enough
/// that a leaked secret with a well-known long prefix (bearer tokens,
/// JWTs, SDK keys ≥ ~40 chars) is truncated at the source even before
/// [`redact_secret_shapes`] runs.
pub(super) const DIAGNOSTIC_MAX_LEN: usize = 160;

/// Sanitize and length-cap a stderr byte buffer for use in an
/// operator-facing diagnostic. Guarantees:
///
/// * ANSI SGR / CSI escape sequences are stripped (`ESC [ … m` / `ESC ]`).
/// * ASCII control bytes other than tab are dropped (no NUL, no BEL,
///   no CR/LF churn inside the single-line excerpt).
/// * Well-known secret-shaped tokens (`sk-…`, `xoxb-…`, `Bearer …`,
///   `ghp_…` / `gho_…`, JWT `eyJ…`, long hex/base64 runs) are replaced
///   with `<REDACTED>` sentinels so a misbehaving CLI cannot leak a
///   credential into the desktop log.
/// * Final length ≤ [`DIAGNOSTIC_MAX_LEN`] bytes, ellipsis included.
fn sanitize_stderr_excerpt(stderr_bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr_bytes);
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let ansi_stripped = strip_ansi(line);
    let control_stripped: String = ansi_stripped
        .chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .collect();
    let redacted = redact_secret_shapes(&control_stripped);
    bound_diagnostic_text(&redacted)
}

/// Public within the crate so the fallback exec-error path can use the
/// same length contract as `sanitize_stderr_excerpt`.
fn bound_diagnostic_text(input: &str) -> String {
    if input.len() <= DIAGNOSTIC_MAX_LEN {
        return input.to_string();
    }
    // Reserve one byte for the ellipsis character (`…` is 3 bytes in
    // UTF-8; account for it so the FINAL length stays ≤ DIAGNOSTIC_MAX_LEN).
    const ELLIPSIS: &str = "…";
    debug_assert!(ELLIPSIS.len() < DIAGNOSTIC_MAX_LEN);
    let mut end = DIAGNOSTIC_MAX_LEN - ELLIPSIS.len();
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(DIAGNOSTIC_MAX_LEN);
    out.push_str(&input[..end]);
    out.push_str(ELLIPSIS);
    out
}

/// Strip a common subset of ANSI escape sequences: `ESC [ ... <letter>`
/// (CSI, colors + cursor moves) and `ESC ] ... BEL` (OSC). Handles the
/// shapes CLIs actually emit; not a complete parser.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                for next in chars.by_ref() {
                    if next == '\x07' {
                        break;
                    }
                }
            }
            Some(_) | None => {}
        }
    }
    out
}

/// Redact well-known secret shapes with a `<REDACTED>` sentinel. Not a
/// complete secret-detector; the defensive length cap in
/// [`bound_diagnostic_text`] catches longer unknown patterns by
/// truncation. Prefix list covers the credentials most likely to leak
/// from a CLI auth probe stderr (OpenAI/Slack/GitHub/JWT/OAuth bearers)
/// plus long hex/base64 runs (session tokens, opaque IDs).
fn redact_secret_shapes(input: &str) -> String {
    const PREFIX_TOKENS: &[&str] = &["sk-", "xoxb-", "xoxp-", "ghp_", "gho_", "eyJ"];
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Well-known prefixed tokens: prefix + ≥ 20 chars of the
        // token alphabet [A-Za-z0-9_.-].
        if let Some(consumed) = match_prefix_secret(bytes, i, PREFIX_TOKENS) {
            out.push_str("<REDACTED>");
            i += consumed;
            continue;
        }
        // "Bearer <token>": whitespace-then-token shape.
        if let Some(consumed) = match_bearer_secret(bytes, i) {
            out.push_str("<REDACTED>");
            i += consumed;
            continue;
        }
        // Long hex/base64 runs (≥ 40 chars of [A-Za-z0-9]).
        if let Some(consumed) = match_long_opaque_token(bytes, i) {
            out.push_str("<REDACTED>");
            i += consumed;
            continue;
        }
        // Fall through: copy this UTF-8 code point verbatim.
        let ch_len = match bytes[i] {
            b if b < 0x80 => 1,
            b if b < 0xC0 => 1, // malformed continuation — skip one byte
            b if b < 0xE0 => 2,
            b if b < 0xF0 => 3,
            _ => 4,
        };
        let end = (i + ch_len).min(bytes.len());
        // Only push if the slice is valid UTF-8; String::from_utf8_lossy
        // is applied at the outer caller so partial bytes are fine.
        if let Ok(s) = std::str::from_utf8(&bytes[i..end]) {
            out.push_str(s);
        }
        i = end.max(i + 1);
    }
    out
}

fn match_prefix_secret(bytes: &[u8], start: usize, prefixes: &[&str]) -> Option<usize> {
    for prefix in prefixes {
        let p = prefix.as_bytes();
        if bytes.len() < start + p.len() {
            continue;
        }
        if &bytes[start..start + p.len()] != p {
            continue;
        }
        let mut end = start + p.len();
        while end < bytes.len() && is_token_byte(bytes[end]) {
            end += 1;
        }
        if end - start >= p.len() + 20 {
            return Some(end - start);
        }
    }
    None
}

fn match_bearer_secret(bytes: &[u8], start: usize) -> Option<usize> {
    const BEARER: &[u8] = b"Bearer ";
    if bytes.len() < start + BEARER.len() + 20 {
        return None;
    }
    if &bytes[start..start + BEARER.len()] != BEARER {
        return None;
    }
    let mut end = start + BEARER.len();
    while end < bytes.len() && is_token_byte(bytes[end]) {
        end += 1;
    }
    if end - (start + BEARER.len()) >= 20 {
        Some(end - start)
    } else {
        None
    }
}

fn match_long_opaque_token(bytes: &[u8], start: usize) -> Option<usize> {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
        end += 1;
    }
    if end - start >= 40 {
        Some(end - start)
    } else {
        None
    }
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+' | b'/')
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
#[path = "cli_probe_tests.rs"]
mod tests;
