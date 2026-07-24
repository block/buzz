use std::path::Path;

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
    /// The probe command is missing or incompatible with this CLI build
    /// (e.g. older Claude Code treating `auth status` as a prompt).
    Unsupported {
        /// Actionable guidance for the user (typically "run `claude update`").
        diagnostic: String,
    },
}

/// Hint shown when Claude Code is too old to expose `claude auth status`.
pub(crate) const CLAUDE_AUTH_PROBE_UPDATE_HINT: &str = "This Claude Code build doesn’t support `claude auth status`. Run `claude update`, then try again.";

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

/// Whether this probe is Claude's JSON `auth status` command.
///
/// Older Claude Code builds lack the `auth` subcommand and treat
/// `auth status` as a free-form prompt, so callers must validate JSON
/// (`loggedIn`) instead of trusting exit status alone.
pub(crate) fn is_claude_auth_status_probe(probe_args: &[&str]) -> bool {
    probe_args.len() >= 3
        && probe_args[0] == "claude"
        && probe_args[probe_args.len() - 2] == "auth"
        && probe_args[probe_args.len() - 1] == "status"
}

/// Parse Claude's `auth status` JSON for the `loggedIn` boolean.
fn parse_claude_logged_in(stdout: &[u8]) -> Option<bool> {
    let text = String::from_utf8_lossy(stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Prefer a full-document parse; fall back to the first JSON object if the
    // CLI wraps status with banners/logging.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return value.get("loggedIn").and_then(|v| v.as_bool());
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end < start {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]).ok()?;
    value.get("loggedIn").and_then(|v| v.as_bool())
}

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
        Ok(o) => classify_auth_probe_output(probe_args, &o.stdout, &o.stderr, o.status.success()),
        Err(_) => ProbeOutcome::LoggedOut,
    }
}

/// Classify collected probe output into a `ProbeOutcome`.
///
/// Shared between `login_probe` (which has the full `Output`) and the
/// process-level timeout path in `probe_auth_status` (which drains stdout /
/// stderr on background threads and collects them separately).
pub(crate) fn classify_auth_probe_output(
    probe_args: &[&str],
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    exit_success: bool,
) -> ProbeOutcome {
    if is_claude_auth_status_probe(probe_args) {
        return match parse_claude_logged_in(stdout_bytes) {
            Some(true) => ProbeOutcome::LoggedIn,
            Some(false) => ProbeOutcome::LoggedOut,
            None => ProbeOutcome::Unsupported {
                diagnostic: CLAUDE_AUTH_PROBE_UPDATE_HINT.to_string(),
            },
        };
    }

    classify_probe_output(stderr_bytes, exit_success)
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

/// Returns true when Claude's `--help` output does not advertise an `auth`
/// subcommand.
///
/// Older builds omit `auth` and treat `claude auth status` as a free-form
/// prompt (which can hang for tens of seconds). Prefer this cheap help parse
/// over running the status probe during install repair.
pub(crate) fn claude_help_missing_auth_command(help_stdout: &str) -> bool {
    let lower = help_stdout.to_lowercase();
    // Prefer the Commands section when present so option text like
    // `--chrome` / "authentication" prose does not false-positive.
    let commands_section = lower.split("commands:").nth(1).unwrap_or(lower.as_str());
    // Match a commands-list line like `auth  Manage authentication`.
    for line in commands_section.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("auth") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return false;
            }
        }
    }
    true
}

/// Returns true when the Claude binary's `auth status` probe is missing or
/// incompatible (so install should run `claude update`).
///
/// Uses `claude --help` (fast, non-interactive) rather than `auth status`,
/// which older builds may interpret as a prompt and hang on.
pub(crate) fn claude_auth_status_needs_upgrade(
    binary_path: &Path,
    augmented_path: Option<&str>,
) -> bool {
    let mut command = std::process::Command::new(binary_path);
    command.arg("--help");
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    crate::util::configure_no_window(&mut command);

    match command.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Some CLIs print help on stderr.
            claude_help_missing_auth_command(&format!("{stdout}\n{stderr}"))
        }
        // If we cannot run --help, attempt the upgrade path rather than
        // leaving onboarding stuck on a hanging auth probe.
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_auth_probe_output, is_claude_auth_status_probe, ProbeOutcome,
        CLAUDE_AUTH_PROBE_UPDATE_HINT, CONFIG_PARSE_SIGNALS,
    };

    #[test]
    fn detects_claude_auth_status_probe_args() {
        assert!(is_claude_auth_status_probe(&["claude", "auth", "status"]));
        assert!(!is_claude_auth_status_probe(&["codex", "login", "status"]));
        assert!(!is_claude_auth_status_probe(&["claude", "doctor"]));
    }

    #[test]
    fn claude_auth_status_json_logged_in() {
        let stdout = br#"{
  "loggedIn": true,
  "authMethod": "claude.ai",
  "email": "user@example.com"
}"#;
        assert_eq!(
            classify_auth_probe_output(&["claude", "auth", "status"], stdout, b"", true),
            ProbeOutcome::LoggedIn
        );
    }

    #[test]
    fn claude_auth_status_json_logged_out() {
        let stdout = br#"{"loggedIn":false}"#;
        assert_eq!(
            classify_auth_probe_output(&["claude", "auth", "status"], stdout, b"", false),
            ProbeOutcome::LoggedOut
        );
    }

    #[test]
    fn claude_auth_status_prompt_hijack_is_unsupported() {
        // Older Claude Code builds lack `auth` and treat the args as a prompt.
        let stdout = b"I'd be happy to help with authentication in Buzz!";
        let outcome = classify_auth_probe_output(&["claude", "auth", "status"], stdout, b"", true);
        assert_eq!(
            outcome,
            ProbeOutcome::Unsupported {
                diagnostic: CLAUDE_AUTH_PROBE_UPDATE_HINT.to_string(),
            }
        );
    }

    #[test]
    fn claude_auth_status_empty_success_is_unsupported() {
        let outcome = classify_auth_probe_output(&["claude", "auth", "status"], b"", b"", true);
        assert!(matches!(outcome, ProbeOutcome::Unsupported { .. }));
    }

    #[test]
    fn claude_help_detects_missing_auth_command() {
        let old_help = r#"
Commands:
  doctor                                Check the health of your Claude Code
  install [options] [target]            Install Claude Code native build
  mcp                                   Configure and manage MCP servers
  setup-token                           Set up a long-lived authentication token
  update|upgrade                        Check for updates and install if available
"#;
        assert!(super::claude_help_missing_auth_command(old_help));

        let new_help = r#"
Commands:
  auth                                  Manage authentication
  doctor                                Check the health of your Claude Code
  update|upgrade                        Check for updates and install if available
"#;
        assert!(!super::claude_help_missing_auth_command(new_help));
    }

    #[test]
    fn non_claude_probe_still_trusts_exit_status() {
        assert_eq!(
            classify_auth_probe_output(&["codex", "login", "status"], b"", b"", true),
            ProbeOutcome::LoggedIn
        );
    }

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

    #[cfg(unix)]
    #[test]
    fn login_probe_claude_json_uses_stdout_not_bare_exit() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");

        // Exit 0 with conversational stdout — must NOT count as logged in.
        let script_path = bin_dir.join("claude");
        fs::write(
            &script_path,
            "#!/bin/sh\necho 'Sure, I can help with auth status!'\nexit 0\n",
        )
        .expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let outcome = super::login_probe(&script_path, &["claude", "auth", "status"], None);
        assert!(
            matches!(outcome, ProbeOutcome::Unsupported { .. }),
            "prompt-hijack stdout must be Unsupported; got {outcome:?}"
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

    /// Manual/live validation: run against whatever `claude` is on PATH.
    /// Ignored in CI — opt in with `--ignored`.
    #[test]
    #[ignore = "live PATH probe; run manually after downgrading Claude Code"]
    fn live_claude_auth_probe_against_path_binary() {
        use crate::managed_agents::discover_acp_runtimes;

        let claude = std::process::Command::new("claude")
            .arg("--version")
            .output()
            .expect("claude on PATH");
        println!(
            "claude --version: {}",
            String::from_utf8_lossy(&claude.stdout).trim()
        );

        let help = std::process::Command::new("claude")
            .arg("--help")
            .output()
            .expect("claude --help");
        let help_text = format!(
            "{}{}",
            String::from_utf8_lossy(&help.stdout),
            String::from_utf8_lossy(&help.stderr)
        );
        let needs_upgrade = super::claude_help_missing_auth_command(&help_text);
        println!("claude_help_missing_auth_command={needs_upgrade}");

        let runtimes = discover_acp_runtimes();
        let entry = runtimes
            .iter()
            .find(|r| r.id == "claude")
            .expect("claude runtime in catalog");
        println!("availability={:?}", entry.availability);
        println!("auth_status={:?}", entry.auth_status);
        println!("login_hint={:?}", entry.login_hint);
        println!("can_auto_install={}", entry.can_auto_install);

        if needs_upgrade {
            match &entry.auth_status {
                crate::managed_agents::AuthStatus::Unknown {
                    diagnostic: Some(diagnostic),
                } if entry.availability
                    == crate::managed_agents::AcpAvailabilityStatus::Available =>
                {
                    assert!(
                        diagnostic.contains("claude update"),
                        "expected update diagnostic, got {diagnostic}"
                    );
                    assert_eq!(entry.login_hint.as_deref(), Some(diagnostic.as_str()));
                }
                other => panic!(
                    "expected Available + Unknown(diagnostic) for outdated Claude, got availability={:?} auth={other:?}",
                    entry.availability
                ),
            }
        }
    }
}
