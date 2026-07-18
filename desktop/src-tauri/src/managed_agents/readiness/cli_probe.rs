use std::path::Path;

use crate::managed_agents::runtime::build_augmented_path;

/// Build the augmented PATH for CLI probes, including nvm's default Node.js
/// bin directory so `#!/usr/bin/env node` shims (e.g. codex-acp) resolve.
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

fn successful_probe_json_outcome(stdout_bytes: &[u8]) -> Option<ProbeOutcome> {
    let parsed = serde_json::from_slice::<serde_json::Value>(stdout_bytes).ok()?;
    let logged_in = parsed
        .get("loggedIn")
        .and_then(serde_json::Value::as_bool)?;
    if !logged_in {
        return Some(ProbeOutcome::LoggedOut);
    }

    let auth_method = parsed
        .get("authMethod")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
        .replace(['-', '_'], "");
    if auth_method == "apikey" {
        return Some(ProbeOutcome::LoggedOut);
    }

    Some(ProbeOutcome::LoggedIn)
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

    match command.output() {
        Ok(o) => classify_probe_output(&o.stdout, &o.stderr, o.status.success()),
        Err(_) => ProbeOutcome::LoggedOut,
    }
}

/// Classify collected probe output into a `ProbeOutcome`.
///
/// Shared between `login_probe` (which has the full `Output`) and the
/// process-level timeout path in `probe_auth_status` (which drains stdout and
/// stderr on background threads and collects them separately).
pub(crate) fn classify_probe_output(
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    exit_success: bool,
) -> ProbeOutcome {
    if exit_success {
        if let Some(outcome) = successful_probe_json_outcome(stdout_bytes) {
            return outcome;
        }
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

    #[test]
    fn successful_json_probe_logged_out_when_logged_in_false() {
        let outcome =
            super::classify_probe_output(br#"{"loggedIn":false,"authMethod":"none"}"#, b"", true);
        assert_eq!(outcome, ProbeOutcome::LoggedOut);
    }

    #[test]
    fn successful_json_probe_logged_out_for_api_key_only_auth() {
        let outcome = super::classify_probe_output(
            br#"{"loggedIn":true,"authMethod":"api_key","apiKeySource":"ANTHROPIC_API_KEY"}"#,
            b"",
            true,
        );
        assert_eq!(outcome, ProbeOutcome::LoggedOut);
    }

    #[test]
    fn successful_json_probe_logged_in_for_account_auth() {
        let outcome = super::classify_probe_output(
            br#"{"loggedIn":true,"authMethod":"claudeai"}"#,
            b"",
            true,
        );
        assert_eq!(outcome, ProbeOutcome::LoggedIn);
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
}
