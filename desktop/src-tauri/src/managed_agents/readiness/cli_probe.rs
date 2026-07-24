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

/// Read Claude's local OAuth state on Windows instead of launching
/// `claude auth status`.
///
/// Claude's native Windows CLI creates console-mode descendants even when the
/// immediate probe process uses `CREATE_NO_WINDOW`, which makes Command Prompt
/// windows flash whenever the desktop refreshes agent readiness. The
/// credentials file contains refreshable OAuth state, so checking for a
/// non-empty access or refresh token provides the same logged-in signal without
/// launching a process.
#[cfg(windows)]
pub(crate) fn native_credentials_probe(
    binary_path: &Path,
    probe_args: &[&str],
) -> Option<ProbeOutcome> {
    let binary_name = binary_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    if binary_name.eq_ignore_ascii_case("claude") && probe_args == ["claude", "auth", "status"] {
        let credentials_path = dirs::home_dir()?.join(".claude").join(".credentials.json");
        return classify_claude_credentials(&std::fs::read(credentials_path).ok()?);
    }

    if binary_name.eq_ignore_ascii_case("codex") && probe_args == ["codex", "login", "status"] {
        let credentials_path = dirs::home_dir()?.join(".codex").join("auth.json");
        return classify_codex_credentials(&std::fs::read(credentials_path).ok()?);
    }

    None
}

#[cfg(any(windows, test))]
fn classify_claude_credentials(bytes: &[u8]) -> Option<ProbeOutcome> {
    let credentials: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let oauth = credentials.get("claudeAiOauth")?;
    let has_token = ["accessToken", "refreshToken"].iter().any(|key| {
        oauth
            .get(key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    });
    Some(if has_token {
        ProbeOutcome::LoggedIn
    } else {
        ProbeOutcome::LoggedOut
    })
}

#[cfg(any(windows, test))]
fn classify_codex_credentials(bytes: &[u8]) -> Option<ProbeOutcome> {
    let credentials: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let has_api_key = credentials
        .get("OPENAI_API_KEY")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    let has_token = credentials.get("tokens").is_some_and(|tokens| {
        ["access_token", "refresh_token", "id_token"]
            .iter()
            .any(|key| {
                tokens
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
            })
    });
    Some(if has_api_key || has_token {
        ProbeOutcome::LoggedIn
    } else {
        ProbeOutcome::LoggedOut
    })
}

#[cfg(not(windows))]
pub(crate) fn native_credentials_probe(
    _binary_path: &Path,
    _probe_args: &[&str],
) -> Option<ProbeOutcome> {
    None
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
    if let Some(outcome) = native_credentials_probe(binary_path, probe_args) {
        return outcome;
    }

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
    use super::{
        classify_claude_credentials, classify_codex_credentials, ProbeOutcome, CONFIG_PARSE_SIGNALS,
    };

    #[test]
    fn claude_credentials_accept_access_or_refresh_token() {
        for credentials in [
            br#"{"claudeAiOauth":{"accessToken":"access","refreshToken":""}}"#.as_slice(),
            br#"{"claudeAiOauth":{"accessToken":"","refreshToken":"refresh"}}"#.as_slice(),
        ] {
            assert_eq!(
                classify_claude_credentials(credentials),
                Some(ProbeOutcome::LoggedIn)
            );
        }
    }

    #[test]
    fn claude_credentials_reject_missing_or_empty_tokens() {
        for credentials in [
            br#"{"claudeAiOauth":{"accessToken":"","refreshToken":""}}"#.as_slice(),
            br#"{"other":{}}"#.as_slice(),
            b"not-json".as_slice(),
        ] {
            assert_ne!(
                classify_claude_credentials(credentials),
                Some(ProbeOutcome::LoggedIn)
            );
        }
    }

    #[test]
    fn codex_credentials_accept_api_key_or_refreshable_tokens() {
        for credentials in [
            br#"{"OPENAI_API_KEY":"key"}"#.as_slice(),
            br#"{"tokens":{"access_token":"access"}}"#.as_slice(),
            br#"{"tokens":{"refresh_token":"refresh"}}"#.as_slice(),
        ] {
            assert_eq!(
                classify_codex_credentials(credentials),
                Some(ProbeOutcome::LoggedIn)
            );
        }
    }

    #[test]
    fn codex_credentials_reject_missing_or_empty_tokens() {
        for credentials in [
            br#"{"OPENAI_API_KEY":"","tokens":{"access_token":""}}"#.as_slice(),
            br#"{"other":{}}"#.as_slice(),
            b"not-json".as_slice(),
        ] {
            assert_ne!(
                classify_codex_credentials(credentials),
                Some(ProbeOutcome::LoggedIn)
            );
        }
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
