//! Shared git subprocess plumbing for the project commands.
//!
//! Runs the system `git` with an ephemeral, env-only auth configuration:
//! the identity nsec is handed to `git-credential-nostr` via environment
//! variables so nothing key-related ever touches disk or global git config.

use crate::{app_state::AppState, managed_agents::resolve_command};
use nostr::{Keys, ToBech32};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use url::Url;

/// Wall-clock cap for a single git invocation. Remote operations talk to
/// relay-supplied clone URLs, so a slow or adversarial remote must not pin
/// `spawn_blocking` threads indefinitely.
const LOCAL_GIT_TIMEOUT: Duration = Duration::from_secs(60);
const REMOTE_GIT_TIMEOUT: Duration = Duration::from_secs(300);

fn git_subcommand<'a>(args: &'a [&str]) -> Option<&'a str> {
    let mut index = 0;
    while let Some(argument) = args.get(index).copied() {
        match argument {
            "-c" | "--config" | "-C" | "--git-dir" | "--work-tree" => index += 2,
            "--no-pager" | "--paginate" | "--end-of-options" => index += 1,
            argument
                if argument.starts_with("--config=")
                    || argument.starts_with("--git-dir=")
                    || argument.starts_with("--work-tree=") =>
            {
                index += 1;
            }
            argument if argument.starts_with('-') => index += 1,
            subcommand => return Some(subcommand),
        }
    }
    None
}

fn git_needs_credentials(args: &[&str]) -> bool {
    matches!(
        git_subcommand(args),
        Some("clone" | "fetch" | "push" | "pull" | "ls-remote" | "merge")
    )
}

pub(crate) struct GitAuthConfig {
    git_path: std::path::PathBuf,
    credential_helper: Option<std::path::PathBuf>,
    nsec: String,
}

fn read_pipe_lossy(pipe: Option<impl Read>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).to_string()
}

pub(crate) fn run_git(
    args: &[&str],
    cwd: Option<&std::path::Path>,
    auth: &GitAuthConfig,
) -> Result<String, String> {
    let mut command = Command::new(&auth.git_path);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let needs_credentials = git_needs_credentials(args);
    let timeout = if needs_credentials {
        REMOTE_GIT_TIMEOUT
    } else {
        LOCAL_GIT_TIMEOUT
    };
    configure_git_auth(&mut command, auth, needs_credentials);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to run git: {error}"))?;

    // Drain the pipes on background threads so a chatty git process can't
    // deadlock on a full pipe while we poll for exit below.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || read_pipe_lossy(stdout_pipe));
    let stderr_thread = std::thread::spawn(move || read_pipe_lossy(stderr_pipe));

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Err(format!("git timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to wait for git: {error}"));
            }
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        let stderr = stderr.trim().to_string();
        return Err(if stderr.is_empty() {
            format!("git exited with status {status}")
        } else {
            stderr
        });
    }
    Ok(stdout)
}

fn configure_git_auth(command: &mut Command, auth: &GitAuthConfig, needs_credentials: bool) {
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_SSH_COMMAND",
        "GIT_EXTERNAL_DIFF",
    ] {
        command.env_remove(key);
    }
    // Git for Windows maps `/dev/null` to `NUL` internally, so this value
    // disables the global config file on every platform.
    command.env("GIT_CONFIG_GLOBAL", "/dev/null");

    // Base entries: disable any inherited credential helper, and neutralize
    // repo-local hooks — every process git spawns inherits our environment
    // (including NOSTR_PRIVATE_KEY below), and a cloned repository's hooks
    // must never run with the identity key in reach.
    let mut entries: Vec<(&str, String)> = vec![
        ("credential.helper", String::new()),
        ("core.hooksPath", "/dev/null".to_string()),
        ("core.fsmonitor", "false".to_string()),
        ("protocol.allow", "never".to_string()),
        ("protocol.http.allow", "always".to_string()),
        ("protocol.https.allow", "always".to_string()),
        ("protocol.ext.allow", "never".to_string()),
        ("protocol.file.allow", "never".to_string()),
    ];
    if needs_credentials {
        let Some(cred_helper) = &auth.credential_helper else {
            return apply_git_config(command, &entries);
        };
        command.env("NOSTR_PRIVATE_KEY", &auth.nsec);
        entries.push(("credential.helper", cred_helper.display().to_string()));
        entries.push(("credential.useHttpPath", "true".to_string()));
    }
    apply_git_config(command, &entries);
}

fn apply_git_config(command: &mut Command, entries: &[(&str, String)]) {
    command.env("GIT_CONFIG_COUNT", entries.len().to_string());
    for (index, (key, value)) in entries.iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
}

pub(crate) fn build_git_auth_config(state: &AppState) -> Result<GitAuthConfig, String> {
    let keys = state.signing_keys()?;
    build_git_auth_config_for_keys(&keys)
}

pub(crate) fn build_git_auth_config_for_keys(keys: &Keys) -> Result<GitAuthConfig, String> {
    let git_path = resolve_command("git").ok_or_else(|| "git was not found on PATH".to_string())?;
    let credential_helper = resolve_command("git-credential-nostr");
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|error| format!("encode identity key: {error}"))?;
    Ok(GitAuthConfig {
        git_path,
        credential_helper,
        nsec,
    })
}

/// Normalizes and validates a relay-supplied branch name. Strips a
/// `refs/heads/` prefix, then rejects anything outside a conservative
/// character allowlist, path traversal (`..`), leading/trailing `/`, and
/// flag-shaped values (leading `-`) so a branch can never reach git as an
/// option instead of a positional argument.
pub(crate) fn clean_branch(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_start_matches("refs/heads/"))
        .filter(|value| {
            !value.is_empty()
                && !value.starts_with('-')
                && !value.contains("..")
                && !value.starts_with('/')
                && !value.ends_with('/')
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
        })
        .map(ToString::to_string)
}

pub(crate) fn validate_clone_url(clone_url: &str) -> Result<(), String> {
    let parsed = Url::parse(clone_url).map_err(|error| format!("invalid clone URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("clone URL must be http or https".into());
    }
    // Buzz git remotes are served at `…/git/<owner-pubkey>/<repo-id>` — a
    // literal `git` segment followed by the 64-hex owner pubkey and a
    // non-empty repository id (the relay may live under a path prefix).
    let segments = parsed
        .path_segments()
        .map(|segments| segments.filter(|s| !s.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    let is_buzz_repo_path = segments
        .iter()
        .rposition(|segment| *segment == "git")
        .filter(|index| segments.len() == index + 3)
        .map(|index| {
            segments[index + 1].len() == 64
                && segments[index + 1].chars().all(|c| c.is_ascii_hexdigit())
                && !segments[index + 2].is_empty()
        })
        .unwrap_or(false);
    if !is_buzz_repo_path {
        return Err("clone URL must point at a Buzz git repository".into());
    }
    Ok(())
}

pub(crate) fn clone_url_owner(clone_url: &str) -> Option<String> {
    let parsed = Url::parse(clone_url).ok()?;
    let segments = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let index = segments.iter().rposition(|segment| *segment == "git")?;
    (segments.len() == index + 3).then(|| segments[index + 1].to_ascii_lowercase())
}

pub(crate) fn validate_workspace_clone_url(
    clone_url: &str,
    state: &AppState,
) -> Result<(), String> {
    let relay_base = crate::relay::relay_api_base_url_with_override(state);
    validate_clone_url_against_relay(clone_url, &relay_base)
}

fn validate_clone_url_against_relay(clone_url: &str, relay_base: &str) -> Result<(), String> {
    validate_clone_url(clone_url)?;
    let clone = Url::parse(clone_url).map_err(|error| format!("invalid clone URL: {error}"))?;
    let relay = Url::parse(relay_base)
        .map_err(|error| format!("configured relay URL is invalid: {error}"))?;
    if clone.scheme() != relay.scheme()
        || clone.host_str() != relay.host_str()
        || clone.port_or_known_default() != relay.port_or_known_default()
    {
        return Err("clone URL must use the active workspace relay".into());
    }
    let relay_path = relay.path().trim_end_matches('/');
    if !relay_path.is_empty() && !clone.path().starts_with(&format!("{relay_path}/")) {
        return Err("clone URL must use the active workspace relay path".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        clean_branch, git_needs_credentials, git_subcommand, validate_clone_url,
        validate_clone_url_against_relay,
    };

    #[test]
    fn git_subcommand_skips_global_config_options() {
        assert_eq!(
            git_subcommand(&[
                "-c",
                "user.name=Buzz User",
                "-c",
                "user.email=user@example.com",
                "merge",
                "HEAD",
            ]),
            Some("merge")
        );
        assert_eq!(
            git_subcommand(&["--config=credential.useHttpPath=true", "fetch", "origin"]),
            Some("fetch")
        );
    }

    #[test]
    fn remote_and_promisor_operations_receive_credentials() {
        assert!(git_needs_credentials(&["fetch", "origin"]));
        assert!(git_needs_credentials(&[
            "-c",
            "user.name=Buzz User",
            "merge",
            "HEAD"
        ]));
        assert!(!git_needs_credentials(&["rev-parse", "HEAD"]));
    }

    #[test]
    fn clean_branch_accepts_plain_and_prefixed_names() {
        assert_eq!(
            clean_branch(Some("refs/heads/feature/x-1".into())),
            Some("feature/x-1".to_string())
        );
        assert_eq!(
            clean_branch(Some(" main ".into())),
            Some("main".to_string())
        );
    }

    #[test]
    fn clean_branch_rejects_flag_shaped_and_traversal_values() {
        assert_eq!(clean_branch(Some("--upload-pack=/tmp/evil".into())), None);
        assert_eq!(clean_branch(Some("-x".into())), None);
        assert_eq!(clean_branch(Some("a/../b".into())), None);
        assert_eq!(clean_branch(Some("/leading".into())), None);
        assert_eq!(clean_branch(Some("trailing/".into())), None);
        assert_eq!(clean_branch(Some("bad name".into())), None);
        assert_eq!(clean_branch(None), None);
    }

    #[test]
    fn validate_clone_url_requires_buzz_repo_shape() {
        let owner = "a".repeat(64);
        assert!(validate_clone_url(&format!("https://relay.example/git/{owner}/repo")).is_ok());
        assert!(
            validate_clone_url(&format!("https://relay.example/prefix/git/{owner}/repo")).is_ok()
        );
        assert!(validate_clone_url("https://relay.example/git/short/repo").is_err());
        assert!(validate_clone_url("https://evil.example/has/git/inpath").is_err());
        assert!(validate_clone_url(&format!("ssh://relay.example/git/{owner}/repo")).is_err());
        assert!(validate_clone_url(&format!(
            "https://relay.example/git/{owner}/repo/unexpected"
        ))
        .is_err());
    }

    #[test]
    fn workspace_clone_url_requires_exact_relay_origin_and_prefix() {
        let owner = "a".repeat(64);
        let valid = format!("https://relay.example/prefix/git/{owner}/repo");
        assert!(validate_clone_url_against_relay(&valid, "https://relay.example/prefix").is_ok());
        assert!(validate_clone_url_against_relay(&valid, "http://relay.example/prefix").is_err());
        assert!(
            validate_clone_url_against_relay(&valid, "https://relay.example:8443/prefix").is_err()
        );
        assert!(validate_clone_url_against_relay(&valid, "https://relay.example/other").is_err());
        assert!(validate_clone_url_against_relay(
            &format!("https://evil.example/prefix/git/{owner}/repo"),
            "https://relay.example/prefix",
        )
        .is_err());
    }
}
