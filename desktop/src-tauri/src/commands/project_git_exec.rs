//! Shared git subprocess plumbing for the project commands.
//!
//! Runs the system `git` with an ephemeral, env-only auth configuration:
//! the identity nsec is handed to `git-credential-nostr` via environment
//! variables so nothing key-related ever touches disk or global git config.

use crate::{
    app_state::AppState,
    commands::{
        project_git_merge_error::ProjectPullRequestMergeError,
        project_github_pull_request::{GhRunner, GitHubRepoRef},
        project_github_repository_state::remap_state_error,
    },
    managed_agents::resolve_command,
};
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

#[derive(Debug)]
pub(crate) struct GitAuthConfig {
    git_path: std::path::PathBuf,
    credential_helper: Option<String>,
    nsec: String,
    allow_file_transport: bool,
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
    crate::util::configure_no_window(&mut command);

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
    command.env_remove("NOSTR_PRIVATE_KEY");
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
        ("protocol.ssh.allow", "always".to_string()),
        ("protocol.ext.allow", "never".to_string()),
        (
            "protocol.file.allow",
            if auth.allow_file_transport {
                "always"
            } else {
                "never"
            }
            .to_string(),
        ),
    ];
    if needs_credentials {
        let Some(credential_helper) = &auth.credential_helper else {
            return apply_git_config(command, &entries);
        };
        if !auth.nsec.is_empty() {
            command.env("NOSTR_PRIVATE_KEY", &auth.nsec);
        }
        entries.push(("credential.helper", credential_helper.clone()));
        entries.push(("credential.useHttpPath", "true".to_string()));
    }
    apply_git_config(command, &entries);
}

/// Format a path for git `credential.helper`.
///
/// Git for Windows invokes helpers via MinGW bash, which treats `\` as
/// escapes. Forward slashes work on every platform git supports.
fn credential_helper_config_value(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn github_credential_helper_config_value(path: &std::path::Path) -> String {
    let path = credential_helper_config_value(path);
    format!("!'{}' auth git-credential", path.replace('\'', "'\\''"),)
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

/// Serialize a GitHub CLI error after remapping merge-failed codes.
pub(crate) fn github_command_error_string(error: ProjectPullRequestMergeError) -> String {
    let remapped = remap_state_error(error, "");
    serde_json::to_string(&remapped).unwrap_or_else(|_| {
        "{\"code\":\"github_state_failed\",\"message\":\"GitHub authentication failed.\",\"recovery\":null}"
            .to_string()
    })
}

/// Build GitHub git auth using an injected `gh` discovery function.
pub(crate) fn build_github_git_auth_config_with<F>(
    clone_url: &str,
    discover: F,
) -> Result<GitAuthConfig, String>
where
    F: FnOnce() -> Result<GhRunner, ProjectPullRequestMergeError>,
{
    let mut auth = credential_free_git_auth_config()?;
    if !clone_url.starts_with("https://github.com/") {
        return Ok(auth);
    }
    let gh = discover().map_err(github_command_error_string)?;
    gh.ensure_auth().map_err(github_command_error_string)?;
    auth.credential_helper = Some(github_credential_helper_config_value(gh.binary_path()));
    Ok(auth)
}

/// Git auth with no identity key and no credential helper. Use it for any
/// remote that is not the active workspace relay.
fn credential_free_git_auth_config() -> Result<GitAuthConfig, String> {
    let git_path = resolve_command("git").ok_or_else(|| "git was not found on PATH".to_string())?;
    Ok(GitAuthConfig {
        git_path,
        credential_helper: None,
        nsec: String::new(),
        allow_file_transport: false,
    })
}

/// Build git auth for read-only browsing (snapshot, file content, diff).
///
/// A remote on the active workspace relay gets the Buzz identity credentials.
/// Any other https, ssh, or scp-style remote gets no credentials, so the nsec
/// and the relay credential helper never reach a third-party host. SSH
/// remotes authenticate through the user's ssh-agent and `~/.ssh/config`.
pub(crate) fn build_read_git_auth_config(
    clone_url: &str,
    state: &AppState,
) -> Result<GitAuthConfig, String> {
    let relay_base = crate::relay::relay_api_base_url_with_override(state);
    if validate_clone_url_against_relay(clone_url, &relay_base).is_ok() {
        return build_git_auth_config(state);
    }
    validate_external_clone_url(clone_url)?;
    credential_free_git_auth_config()
}

/// Build host-aware git auth for Buzz remotes or github.com remotes.
pub(crate) fn build_git_operation_auth_config(
    clone_url: &str,
    state: &AppState,
) -> Result<GitAuthConfig, String> {
    if GitHubRepoRef::parse(clone_url).is_ok() {
        return build_github_git_auth_config_with(clone_url, GhRunner::discover);
    }
    build_git_auth_config(state)
}

pub(crate) fn build_git_clone_auth_config(
    clone_url: &str,
    state: &AppState,
) -> Result<GitAuthConfig, String> {
    build_git_operation_auth_config(clone_url, state)
}

pub(crate) fn build_git_auth_config_for_keys(keys: &Keys) -> Result<GitAuthConfig, String> {
    let git_path = resolve_command("git").ok_or_else(|| "git was not found on PATH".to_string())?;
    let credential_helper =
        resolve_command("git-credential-nostr").map(|path| credential_helper_config_value(&path));
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|error| format!("encode identity key: {error}"))?;
    Ok(GitAuthConfig {
        git_path,
        credential_helper,
        nsec,
        allow_file_transport: false,
    })
}

#[cfg(test)]
pub(crate) fn build_test_git_auth_config() -> Result<GitAuthConfig, String> {
    let mut auth = build_git_auth_config_for_keys(&Keys::generate())?;
    auth.allow_file_transport = true;
    Ok(auth)
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

pub(crate) fn clean_target_ref(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_string();
    for prefix in ["refs/tags/", "refs/nostr/"] {
        if let Some(name) = value.strip_prefix(prefix) {
            let clean_name = clean_branch(Some(name.to_string()))?;
            return (clean_name == name).then_some(format!("{prefix}{clean_name}"));
        }
    }
    None
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

/// Accept an `https://`, `ssh://`, or scp-style (`user@host:path`) git URL
/// on any host. Git receives the URL as a positional argument, so the URL
/// must not start with `-`, and no host, user, or path part may start with
/// `-` either (ssh option injection). Embedded https credentials are refused.
pub(crate) fn validate_external_clone_url(clone_url: &str) -> Result<(), String> {
    const INVALID: &str = "clone URL must be an https://, ssh://, or user@host:path git URL";
    let is_safe_part = |part: &str, extra: &[char]| {
        !part.is_empty()
            && !part.starts_with('-')
            && part.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') || extra.contains(&c)
            })
    };
    if clone_url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(INVALID.into());
    }
    let (user, host, path) = match Url::parse(clone_url) {
        Ok(parsed) => {
            let allowed = match parsed.scheme() {
                "https" => parsed.username().is_empty() && parsed.password().is_none(),
                "ssh" => parsed.password().is_none(),
                _ => false,
            };
            if !allowed {
                return Err(INVALID.into());
            }
            (
                parsed.username().to_string(),
                parsed.host_str().unwrap_or_default().to_string(),
                parsed.path().trim_start_matches('/').to_string(),
            )
        }
        Err(_) => {
            let (user_host, path) = clone_url.split_once(':').ok_or(INVALID)?;
            let (user, host) = user_host.split_once('@').ok_or(INVALID)?;
            if user.is_empty() {
                return Err(INVALID.into());
            }
            (user.to_string(), host.to_string(), path.to_string())
        }
    };
    if (!user.is_empty() && !is_safe_part(&user, &[]))
        || !is_safe_part(&host, &[])
        || !is_safe_part(&path, &['/', '~'])
    {
        return Err(INVALID.into());
    }
    Ok(())
}

fn validate_github_clone_url(clone_url: &str) -> Result<(), String> {
    GitHubRepoRef::parse(clone_url).map(|_| ())
}

pub(crate) fn validate_local_clone_url(clone_url: &str) -> Result<(), String> {
    if validate_clone_url(clone_url).is_ok() || validate_github_clone_url(clone_url).is_ok() {
        return Ok(());
    }
    Err("clone URL must point at a Buzz repository or GitHub repository".into())
}

pub(crate) fn validate_local_clone_url_for_workspace(
    clone_url: &str,
    state: &AppState,
) -> Result<(), String> {
    if validate_github_clone_url(clone_url).is_ok() {
        return Ok(());
    }
    validate_workspace_clone_url(clone_url, state)
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

/// Accept a Buzz URL on the active relay or a strict github.com URL.
pub(crate) fn validate_git_operation_url(clone_url: &str, state: &AppState) -> Result<(), String> {
    let relay_base = crate::relay::relay_api_base_url_with_override(state);
    validate_git_operation_url_against_relay(clone_url, &relay_base)
}

fn validate_git_operation_url_against_relay(
    clone_url: &str,
    relay_base: &str,
) -> Result<(), String> {
    if GitHubRepoRef::parse(clone_url).is_ok() {
        return Ok(());
    }
    validate_clone_url_against_relay(clone_url, relay_base)
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
        build_github_git_auth_config_with, clean_branch, clean_target_ref, configure_git_auth,
        credential_free_git_auth_config, credential_helper_config_value, git_needs_credentials,
        git_subcommand, github_command_error_string, github_credential_helper_config_value,
        validate_clone_url, validate_clone_url_against_relay, validate_external_clone_url,
        validate_git_operation_url_against_relay, validate_local_clone_url, GhRunner,
        GitAuthConfig, ProjectPullRequestMergeError,
    };

    #[test]
    fn external_clone_url_accepts_https_ssh_and_scp_style() {
        for clone_url in [
            "git@git.oceanlabs.dev:vtc/3q-chess/3q-chess-master.git",
            "git@gitlab.com:~user/repo.git",
            "ssh://git@git.example.com:2222/team/app.git",
            "https://git.example.com/team/app.git",
            "https://gitlab.com/acme/app",
        ] {
            assert!(
                validate_external_clone_url(clone_url).is_ok(),
                "{clone_url}"
            );
        }
    }

    #[test]
    fn external_clone_url_rejects_unsafe_shapes() {
        for clone_url in [
            "-uhost:path",
            "git@-oProxyCommand=evil:path",
            "-oProxyCommand=evil@host:path",
            "git@host:-path",
            "ssh://-oProxyCommand=evil/repo",
            "ssh://git@host/-repo",
            "https://token@git.example.com/team/app.git",
            "https://user:secret@git.example.com/team/app.git",
            "http://git.example.com/team/app.git",
            "file:///etc/passwd",
            "ext::sh -c evil",
            "host:path",
            "@host:path",
            "git@host:",
            "git@host:team/app .git",
            "/local/path",
        ] {
            assert!(
                validate_external_clone_url(clone_url).is_err(),
                "{clone_url}"
            );
        }
    }

    #[test]
    fn credential_free_auth_never_exposes_identity_or_helper() {
        let auth = credential_free_git_auth_config().expect("git on PATH");
        assert!(auth.nsec.is_empty());
        assert!(auth.credential_helper.is_none());
        assert!(!auth.allow_file_transport);

        let mut command = std::process::Command::new(&auth.git_path);
        command.env("NOSTR_PRIVATE_KEY", "must-not-leak");
        configure_git_auth(&mut command, &auth, true);
        let envs: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&("NOSTR_PRIVATE_KEY".to_string(), None)));
        let helper_values: Vec<&str> = envs
            .iter()
            .filter(|(key, value)| {
                key.starts_with("GIT_CONFIG_KEY_") && value.as_deref() == Some("credential.helper")
            })
            .filter_map(|(key, _)| {
                let value_key = key.replace("GIT_CONFIG_KEY_", "GIT_CONFIG_VALUE_");
                envs.iter()
                    .find(|(candidate, _)| *candidate == value_key)
                    .and_then(|(_, value)| value.as_deref())
            })
            .collect();
        assert_eq!(helper_values, vec![""]);
    }

    #[test]
    fn credential_helper_config_value_uses_forward_slashes() {
        let path =
            std::path::PathBuf::from(r"C:\Users\x\AppData\Local\Buzz\git-credential-nostr.exe");
        assert_eq!(
            credential_helper_config_value(&path),
            "C:/Users/x/AppData/Local/Buzz/git-credential-nostr.exe",
        );
    }

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
    fn clean_target_ref_accepts_only_tags_and_pull_request_refs() {
        assert_eq!(
            clean_target_ref(Some("refs/tags/v1.0.0".into())),
            Some("refs/tags/v1.0.0".to_string())
        );
        assert_eq!(
            clean_target_ref(Some("refs/nostr/abc123".into())),
            Some("refs/nostr/abc123".to_string())
        );
        assert_eq!(clean_target_ref(Some("refs/heads/main".into())), None);
        assert_eq!(clean_target_ref(Some("refs/tags/../main".into())), None);
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

    #[test]
    fn git_operation_url_accepts_strict_github_https_and_ssh() {
        let relay = "https://relay.example/prefix";
        for clone_url in [
            "https://github.com/acme/app",
            "https://github.com/acme/app.git",
            "git@github.com:acme/app.git",
            "ssh://git@github.com/acme/app.git",
        ] {
            assert!(
                validate_git_operation_url_against_relay(clone_url, relay).is_ok(),
                "{clone_url}",
            );
        }
    }

    #[test]
    fn git_operation_url_rejects_other_hosts_and_keeps_buzz_relay_scoped() {
        let owner = "a".repeat(64);
        let relay = "https://relay.example/prefix";
        assert!(
            validate_git_operation_url_against_relay("https://gitlab.com/acme/app", relay).is_err()
        );
        assert!(validate_git_operation_url_against_relay(
            "https://github.com/acme/app/issues",
            relay,
        )
        .is_err());
        assert!(validate_git_operation_url_against_relay(
            &format!("https://relay.example/prefix/git/{owner}/repo"),
            relay,
        )
        .is_ok());
        assert!(validate_git_operation_url_against_relay(
            &format!("https://evil.example/prefix/git/{owner}/repo"),
            relay,
        )
        .is_err());
    }

    #[test]
    fn local_clone_url_allows_github_https_and_ssh_urls() {
        assert!(validate_local_clone_url("https://github.com/block/buzz").is_ok());
        assert!(validate_local_clone_url("https://github.com/block/buzz.git").is_ok());
        assert!(validate_local_clone_url("ssh://git@github.com/block/buzz").is_ok());
        assert!(validate_local_clone_url("ssh://git@github.com/block/buzz.git").is_ok());
        assert!(validate_local_clone_url("git@github.com:block/buzz.git").is_ok());
        assert!(validate_local_clone_url("http://github.com/block/buzz").is_err());
        assert!(validate_local_clone_url("https://github.com/block/buzz/issues").is_err());
        assert!(validate_local_clone_url("https://user@github.com/block/buzz").is_err());
        assert!(validate_local_clone_url("ssh://github.com/block/buzz").is_err());
        assert!(validate_local_clone_url("ssh://git@github.com/block/buzz/issues").is_err());
        assert!(validate_local_clone_url("git@github.com:block/buzz/issues").is_err());
        assert!(validate_local_clone_url("https://github.com.evil.test/block/buzz").is_err());
        assert!(validate_local_clone_url("https://gitlab.com/block/buzz").is_err());
    }

    fn error_json_code(error: &str) -> String {
        serde_json::from_str::<serde_json::Value>(error)
            .ok()
            .and_then(|value| value.get("code")?.as_str().map(ToString::to_string))
            .unwrap_or_default()
    }

    #[test]
    fn github_git_auth_quotes_spaces_and_apostrophes_in_helper_path() {
        let path = std::path::Path::new("/tmp/Buzz's GitHub CLI/gh");
        assert_eq!(
            github_credential_helper_config_value(path),
            "!'/tmp/Buzz'\\''s GitHub CLI/gh' auth git-credential",
        );
    }

    #[cfg(unix)]
    fn fake_gh(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("create fake gh directory");
        let bin_dir = dir.path().join("GitHub CLI");
        std::fs::create_dir(&bin_dir).expect("create spaced bin directory");
        let path = bin_dir.join("gh");
        std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{script}\n")).expect("write fake gh");
        let mut permissions = std::fs::metadata(&path)
            .expect("stat fake gh")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).expect("chmod fake gh");
        (dir, path)
    }

    #[cfg(unix)]
    fn invoke_credential_helper(auth: &GitAuthConfig) -> std::process::ExitStatus {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut command = Command::new(&auth.git_path);
        command
            .args(["credential", "fill"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("NOSTR_PRIVATE_KEY", "must-not-leak");
        configure_git_auth(&mut command, auth, true);
        let mut child = command.spawn().expect("run git credential fill");
        let mut stdin = child.stdin.take().expect("credential stdin");
        stdin
            .write_all(b"protocol=https\nhost=github.com\n\n")
            .expect("write credential request");
        drop(stdin);
        child.wait().expect("wait for git credential fill")
    }

    #[cfg(unix)]
    #[test]
    fn github_git_auth_https_invokes_quoted_gh_helper_without_nsec() {
        let script = r#"
printf '%s\n' "$*" >> "${0%/GitHub CLI/gh}/calls"
case "$*" in
  'auth status --hostname github.com') exit 0 ;;
  'auth git-credential get')
    if [ "${NOSTR_PRIVATE_KEY+x}" = x ]; then exit 41; fi
    printf 'username=test-user\npassword=test-token\n'
    exit 0
    ;;
  api\ *) exit 42 ;;
  *) exit 43 ;;
esac
"#;
        let (dir, path) = fake_gh(script);
        let auth = build_github_git_auth_config_with("https://github.com/acme/app", || {
            GhRunner::from_resolved(Some(path.clone()))
        })
        .expect("https GitHub auth");
        let helper = auth
            .credential_helper
            .as_deref()
            .expect("credential helper");
        assert!(helper.starts_with("!'"), "{helper}");
        assert!(helper.ends_with("' auth git-credential"), "{helper}");
        assert!(auth.nsec.is_empty());
        let status = invoke_credential_helper(&auth);
        assert!(status.success(), "git credential fill failed: {status}");
        let calls = std::fs::read_to_string(dir.path().join("calls")).expect("read calls");
        assert!(calls
            .lines()
            .any(|line| line == "auth status --hostname github.com"));
        assert!(calls.lines().any(|line| line == "auth git-credential get"));
        assert!(!calls.lines().any(|line| line.starts_with("api ")));
    }

    #[test]
    fn github_git_auth_ssh_skips_gh_helper_and_discovery() {
        let auth = build_github_git_auth_config_with("git@github.com:acme/app.git", || {
            panic!("SSH GitHub must not discover gh")
        })
        .expect("SSH GitHub auth");
        assert!(auth.credential_helper.is_none());
        assert!(auth.nsec.is_empty());
    }

    #[test]
    fn github_git_auth_missing_cli_is_structured_json() {
        let error = build_github_git_auth_config_with("https://github.com/acme/app", || {
            GhRunner::from_resolved(None)
        })
        .expect_err("missing gh must fail");
        assert_eq!(error_json_code(&error), "github_cli_missing");
    }

    #[cfg(unix)]
    #[test]
    fn github_git_auth_failed_status_is_structured_json() {
        let (_dir, path) = fake_gh(
            r#"
case "$*" in
  'auth status --hostname github.com') exit 1 ;;
  *) exit 43 ;;
esac
"#,
        );
        let error = build_github_git_auth_config_with("https://github.com/acme/app", || {
            GhRunner::from_resolved(Some(path))
        })
        .expect_err("failed auth status must fail");
        assert_eq!(error_json_code(&error), "github_auth_required");
    }

    #[test]
    fn github_git_auth_remaps_merge_failed_before_serializing() {
        let json = github_command_error_string(ProjectPullRequestMergeError::new(
            "github_merge_failed",
            "boom",
        ));
        assert_eq!(error_json_code(&json), "github_state_failed");
    }

    #[test]
    fn buzz_git_auth_adds_nsec_only_for_credential_operations() {
        use std::ffi::OsStr;
        use std::process::Command;
        let auth = GitAuthConfig {
            git_path: std::path::PathBuf::from("git"),
            credential_helper: Some("/tmp/git-credential-nostr".to_string()),
            nsec: "nsec1test".to_string(),
            allow_file_transport: false,
        };
        let mut local = Command::new("git");
        configure_git_auth(&mut local, &auth, false);
        let local_nsec = local
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("NOSTR_PRIVATE_KEY"))
            .and_then(|(_, value)| value);
        assert_eq!(local_nsec, None);
        let mut remote = Command::new("git");
        configure_git_auth(&mut remote, &auth, true);
        let remote_nsec = remote
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("NOSTR_PRIVATE_KEY"))
            .and_then(|(_, value)| value);
        assert_eq!(remote_nsec, Some(OsStr::new("nsec1test")));
    }
}
