//! Shared git subprocess plumbing for the project commands.
//!
//! Runs the system `git` with an ephemeral, env-only auth configuration:
//! the identity nsec is handed to `git-credential-nostr` via environment
//! variables so nothing key-related ever touches disk or global git config.
//!
//! External (non-Buzz) remotes are restricted to the github.com built-in
//! plus any exact HTTPS origins the operator lists in
//! `BUZZ_TRUSTED_EXTERNAL_GIT_ORIGINS` (see [`parse_trusted_external_origins`]).
//! Those remotes authenticate through CLI-managed `gh`/`glab` credentials
//! rather than the Buzz identity — never a value read from that env var.

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
        Some(
            "clone"
                | "fetch"
                | "push"
                | "pull"
                | "ls-remote"
                | "merge"
                | "checkout"
                | "show"
                | "diff"
        )
    )
}

pub(crate) struct GitAuthConfig {
    git_path: std::path::PathBuf,
    credential_entries: Vec<(String, String)>,
    nsec: Option<String>,
    allow_file_transport: bool,
    /// Set when an external remote is trusted but its credential helper
    /// (`gh`/`glab`) is not on PATH. The clone/fetch still runs anonymously
    /// (so public repositories keep working); this records which helper to
    /// point the user at if the operation then fails on a private repo.
    missing_credential_helper: Option<&'static str>,
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
        let mut message = if stderr.is_empty() {
            format!("git exited with status {status}")
        } else {
            stderr
        };
        // An external remote with no `gh`/`glab` on PATH runs anonymously
        // (see `missing_credential_helper`), so a private repo fails here
        // rather than at auth-config time. Point the user at the fix instead
        // of surfacing a bare git 401/403.
        if needs_credentials {
            if let Some(helper_name) = auth.missing_credential_helper {
                message.push_str(&format!(
                    "\n\nIf this repository is private, install the {helper_name} CLI, run `{helper_name} auth login`, and restart Buzz before trying again."
                ));
            }
        }
        return Err(message);
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
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_EXEC_PATH",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_ALLOW_PROTOCOL",
        "GIT_PROTOCOL_FROM_USER",
        "NOSTR_PRIVATE_KEY",
        TRUSTED_EXTERNAL_GIT_ORIGINS_ENV,
    ] {
        command.env_remove(key);
    }
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if key.starts_with("GIT_CONFIG_KEY_") || key.starts_with("GIT_CONFIG_VALUE_") {
            command.env_remove(key.as_ref());
        }
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
        if let Some(nsec) = &auth.nsec {
            command.env("NOSTR_PRIVATE_KEY", nsec);
        }
        entries.extend(
            auth.credential_entries
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone())),
        );
    }
    apply_git_config(command, &entries);
}

/// Format a path for git `credential.helper`.
///
/// Git for Windows invokes helpers via MinGW bash, which treats `\` as
/// escapes, so paths need forward slashes there. On POSIX platforms a
/// backslash is an ordinary (if unusual) filename character, so it must be
/// left alone here — blanket-replacing it would corrupt any POSIX path that
/// legitimately contains one.
fn credential_helper_config_value(path: &std::path::Path) -> String {
    let value = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        value.replace('\\', "/")
    } else {
        value
    }
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

/// Builds the auth config to use for network git operations against
/// `clone_url`, which may be a Buzz repository or a trusted external HTTPS
/// remote (see [`external_https_remote`]). Falls back to the viewer's Buzz
/// identity for anything that isn't an external remote.
pub(crate) fn build_git_auth_config_for_url(
    clone_url: &str,
    state: &AppState,
) -> Result<GitAuthConfig, String> {
    if let Some(remote) = external_https_remote(clone_url)? {
        return build_external_auth_for_remote(&remote);
    }
    build_git_auth_config(state)
}

/// As [`build_git_auth_config_for_url`], but for Buzz repositories the auth
/// is scoped to a specific identity (e.g. a managed agent acting as a
/// repository owner) instead of the viewer's own signing keys.
pub(crate) fn build_git_auth_config_for_url_with_keys(
    clone_url: &str,
    keys: &Keys,
) -> Result<GitAuthConfig, String> {
    if let Some(remote) = external_https_remote(clone_url)? {
        return build_external_auth_for_remote(&remote);
    }
    build_git_auth_config_for_keys(keys)
}

/// Missing `gh`/`glab` does not fail here — it degrades to an anonymous
/// clone/fetch (see [`GitAuthConfig::missing_credential_helper`]), so public
/// repositories on a trusted external origin still work without either CLI
/// installed. A private repository then fails naturally at the git call,
/// where `run_git` attaches setup guidance.
fn build_external_auth_for_remote(remote: &ExternalHttpsRemote) -> Result<GitAuthConfig, String> {
    let helper_name = external_helper_name(remote);
    let helper = resolve_command(helper_name);
    build_external_git_auth_config(remote, helper)
}

pub(crate) fn build_git_auth_config_for_keys(keys: &Keys) -> Result<GitAuthConfig, String> {
    let git_path = resolve_command("git").ok_or_else(|| "git was not found on PATH".to_string())?;
    let credential_helper = resolve_command("git-credential-nostr");
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|error| format!("encode identity key: {error}"))?;
    let credential_entries = credential_helper
        .as_ref()
        .map(|helper| {
            vec![
                (
                    // Wrapped in `!'...'` (shell-quoted) rather than assigned
                    // as a bare path: git always runs a slash-containing
                    // `credential.helper` value through the shell, so an
                    // unquoted path containing a space would be split into
                    // multiple argv words.
                    "credential.helper".to_string(),
                    credential_helper_command(helper, &[]),
                ),
                ("credential.useHttpPath".to_string(), "true".to_string()),
            ]
        })
        .unwrap_or_default();
    Ok(GitAuthConfig {
        git_path,
        credential_entries,
        nsec: credential_helper.is_some().then_some(nsec),
        allow_file_transport: false,
        missing_credential_helper: None,
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
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("clone URL must not contain credentials, a query, or fragment".into());
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

struct ExternalHttpsRemote {
    host: String,
    credential_url: String,
}

fn external_helper_name(remote: &ExternalHttpsRemote) -> &'static str {
    if remote.host == "github.com" {
        "gh"
    } else {
        "glab"
    }
}

/// Operator-configured allowlist of additional external git origins, beyond
/// the github.com built-in. A value, not a secret: it names hosts, never
/// credentials — `gh`/`glab` own the actual auth. See
/// [`parse_trusted_external_origins`] for the exact grammar.
const TRUSTED_EXTERNAL_GIT_ORIGINS_ENV: &str = "BUZZ_TRUSTED_EXTERNAL_GIT_ORIGINS";

fn trusted_external_origins_env() -> Vec<String> {
    std::env::var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV)
        .ok()
        .map(|value| parse_trusted_external_origins(&value))
        .unwrap_or_default()
}

/// Parses a comma-separated list of exact HTTPS origins the operator has
/// chosen to trust for external git remotes (e.g.
/// `https://gitlab.onlyarag.com`), in addition to the github.com built-in.
/// Each entry must be a bare origin: https scheme, no userinfo, no path
/// beyond `/`, no query, no fragment. Matching against a clone URL later is
/// by exact origin string (see [`Url::origin`]'s `ascii_serialization`),
/// which normalizes away a default `:443` on both sides — so an origin's
/// explicit non-default port is significant, and an omitted port matches
/// only the HTTPS default. A malformed entry is dropped with a warning
/// rather than failing the whole list, since this is operator configuration
/// rather than attacker input.
fn parse_trusted_external_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let parsed = Url::parse(entry).ok().filter(|parsed| {
                parsed.scheme() == "https"
                    && parsed.host_str().is_some()
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && parsed.query().is_none()
                    && parsed.fragment().is_none()
                    && matches!(parsed.path(), "" | "/")
            });
            let Some(parsed) = parsed else {
                // Do not echo malformed configuration: an operator may have
                // accidentally included URL credentials, and diagnostics must
                // never turn that mistake into a log leak.
                eprintln!(
                    "buzz-desktop: ignoring an invalid {TRUSTED_EXTERNAL_GIT_ORIGINS_ENV} entry"
                );
                return None;
            };
            Some(parsed.origin().ascii_serialization())
        })
        .collect()
}

fn external_https_remote(clone_url: &str) -> Result<Option<ExternalHttpsRemote>, String> {
    external_https_remote_with_trusted(clone_url, &trusted_external_origins_env())
}

fn external_https_remote_with_trusted(
    clone_url: &str,
    trusted_origins: &[String],
) -> Result<Option<ExternalHttpsRemote>, String> {
    if validate_clone_url(clone_url).is_ok() {
        return Ok(None);
    }
    let parsed = Url::parse(clone_url).map_err(|error| format!("invalid clone URL: {error}"))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "external clone URL must use https without credentials, a query, or fragment".into(),
        );
    }
    let raw_path = clone_url
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('/').map(|(_, path)| path))
        .unwrap_or_default();
    if raw_path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err("external clone URL must not contain path traversal".into());
    }
    let mut segments = parsed
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    if segments.last() == Some(&"") {
        // A trailing slash produces one trailing empty segment (e.g.
        // `owner/repo/`); drop it instead of counting it as a path
        // component.
        segments.pop();
    }
    let valid_segment = |segment: &&str| {
        !segment.is_empty()
            && !segment.starts_with('-')
            && !segment.contains("..")
            && segment.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            })
    };
    if segments.len() < 2 || !segments.iter().all(valid_segment) {
        return Err("external clone URL must name a repository with safe path segments".into());
    }
    let host = parsed
        .host_str()
        .expect("checked above")
        .to_ascii_lowercase();
    if host == "github.com" && segments.len() != 2 {
        return Err("GitHub clone URL must name one owner and repository".into());
    }
    let origin = parsed.origin().ascii_serialization();
    let is_trusted = origin == "https://github.com" || trusted_origins.contains(&origin);
    if !is_trusted {
        return Err(
            "external clone URL host must be github.com or a host listed in \
             BUZZ_TRUSTED_EXTERNAL_GIT_ORIGINS"
                .into(),
        );
    }
    Ok(Some(ExternalHttpsRemote {
        host,
        credential_url: origin,
    }))
}

fn external_credential_entries(
    remote: &ExternalHttpsRemote,
    helper: &std::path::Path,
) -> Vec<(String, String)> {
    let scope = format!("credential.{}", remote.credential_url);
    vec![
        (
            format!("{scope}.helper"),
            credential_helper_command(helper, &["auth", "git-credential"]),
        ),
        (format!("{scope}.useHttpPath"), "true".to_string()),
    ]
}

fn build_external_git_auth_config(
    remote: &ExternalHttpsRemote,
    helper: Option<std::path::PathBuf>,
) -> Result<GitAuthConfig, String> {
    let git_path = resolve_command("git").ok_or_else(|| "git was not found on PATH".to_string())?;
    match helper {
        Some(helper) => Ok(GitAuthConfig {
            git_path,
            credential_entries: external_credential_entries(remote, &helper),
            nsec: None,
            allow_file_transport: false,
            missing_credential_helper: None,
        }),
        // No `gh`/`glab` on PATH: proceed anonymously so a public repository
        // still clones/fetches. `run_git` attaches setup guidance if a
        // credentialed operation then fails.
        None => Ok(GitAuthConfig {
            git_path,
            credential_entries: Vec::new(),
            nsec: None,
            allow_file_transport: false,
            missing_credential_helper: Some(external_helper_name(remote)),
        }),
    }
}

/// Builds a `!'<path>' [args...]` credential.helper value. Git runs any
/// slash-containing `credential.helper` value through the shell (appending
/// the get/store/erase operation as the final word), so the path is always
/// single-quoted here — an unquoted path containing a space would otherwise
/// be split into multiple argv words. `args` are trusted literals (fixed
/// gh/glab subcommands), never caller-supplied values.
fn credential_helper_command(path: &std::path::Path, args: &[&str]) -> String {
    let escaped_path = credential_helper_config_value(path).replace('\'', "'\"'\"'");
    let mut command = format!("!'{escaped_path}'");
    for arg in args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

pub(crate) fn validate_local_clone_url(clone_url: &str) -> Result<(), String> {
    if validate_clone_url(clone_url).is_ok() || external_https_remote(clone_url)?.is_some() {
        return Ok(());
    }
    Err("clone URL must point at a Buzz repository or a safe external HTTPS repository".into())
}

pub(crate) fn validate_local_clone_url_for_workspace(
    clone_url: &str,
    state: &AppState,
) -> Result<(), String> {
    if external_https_remote(clone_url)?.is_some() {
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
        build_external_git_auth_config, clean_branch, clean_target_ref, credential_helper_command,
        credential_helper_config_value, external_credential_entries, external_helper_name,
        external_https_remote, external_https_remote_with_trusted, git_needs_credentials,
        git_subcommand, parse_trusted_external_origins, run_git, trusted_external_origins_env,
        validate_clone_url, validate_clone_url_against_relay, validate_local_clone_url,
        TRUSTED_EXTERNAL_GIT_ORIGINS_ENV,
    };

    // Guards tests that mutate `BUZZ_TRUSTED_EXTERNAL_GIT_ORIGINS`: env vars
    // are process-global, so parallel `cargo test` threads must serialize
    // around it (same pattern as `app_state_tests::ENV_LOCK`).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(windows)]
    #[test]
    fn credential_helper_config_value_uses_forward_slashes_on_windows() {
        let path =
            std::path::PathBuf::from(r"C:\Users\x\AppData\Local\Buzz\git-credential-nostr.exe");
        assert_eq!(
            credential_helper_config_value(&path),
            "C:/Users/x/AppData/Local/Buzz/git-credential-nostr.exe",
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn credential_helper_config_value_preserves_posix_backslashes() {
        // A backslash is an ordinary filename character on POSIX; blanket
        // replacement would corrupt a path that legitimately contains one.
        let path = std::path::PathBuf::from("/opt/weird\\name/git-credential-nostr");
        assert_eq!(
            credential_helper_config_value(&path),
            "/opt/weird\\name/git-credential-nostr",
        );
    }

    #[test]
    fn credential_helper_command_has_no_trailing_space_with_no_args() {
        assert_eq!(
            credential_helper_command(
                std::path::Path::new("/usr/local/bin/git-credential-nostr"),
                &[]
            ),
            "!'/usr/local/bin/git-credential-nostr'"
        );
    }

    #[test]
    fn credential_helper_command_escapes_embedded_single_quotes() {
        assert_eq!(
            credential_helper_command(std::path::Path::new("/opt/it's/glab"), &["auth"]),
            "!'/opt/it'\"'\"'s/glab' auth"
        );
    }

    #[test]
    fn external_helpers_are_origin_scoped_and_never_receive_credentials_in_argv() {
        let trusted = ["https://gitlab.onlyarag.com".to_string()];
        let remote = external_https_remote_with_trusted(
            "https://gitlab.onlyarag.com/group/private-repo.git",
            &trusted,
        )
        .unwrap()
        .expect("external remote");
        let entries = external_credential_entries(
            &remote,
            std::path::Path::new("/Applications/GitLab CLI/glab"),
        );
        assert_eq!(
            entries[0].0,
            "credential.https://gitlab.onlyarag.com.helper"
        );
        assert_eq!(
            entries[0].1,
            "!'/Applications/GitLab CLI/glab' auth git-credential"
        );
        assert_eq!(
            entries[1],
            (
                "credential.https://gitlab.onlyarag.com.useHttpPath".into(),
                "true".into()
            )
        );
        assert_eq!(
            credential_helper_command(std::path::Path::new("/bin/gh"), &["auth", "git-credential"]),
            "!'/bin/gh' auth git-credential"
        );
        let github = external_https_remote("https://github.com/block/buzz.git")
            .unwrap()
            .expect("GitHub remote");
        assert_eq!(external_helper_name(&github), "gh");
        assert_eq!(external_helper_name(&remote), "glab");
    }

    #[test]
    fn missing_helper_yields_anonymous_auth_with_setup_hint() {
        let remote = external_https_remote_with_trusted(
            "https://gitlab.onlyarag.com/group/private-repo.git",
            &["https://gitlab.onlyarag.com".to_string()],
        )
        .unwrap()
        .expect("external remote");
        let auth = build_external_git_auth_config(&remote, None).expect("build anonymous auth");
        assert!(auth.credential_entries.is_empty());
        assert!(auth.nsec.is_none());
        assert_eq!(auth.missing_credential_helper, Some("glab"));
    }

    #[test]
    fn run_git_appends_setup_guidance_when_credential_helper_is_missing() {
        let remote = external_https_remote_with_trusted(
            "https://gitlab.onlyarag.com/group/private-repo.git",
            &["https://gitlab.onlyarag.com".to_string()],
        )
        .unwrap()
        .expect("external remote");
        let mut auth = build_external_git_auth_config(&remote, None).expect("build anonymous auth");
        auth.allow_file_transport = true;
        let repo = tempfile::tempdir().expect("create test directory");
        let repo_path = repo.path().to_str().expect("repo path");
        run_git(&["init", "--bare", "--", repo_path], None, &auth).expect("init bare repo");

        let error = run_git(
            &["fetch", "--end-of-options", "origin", "main"],
            Some(repo.path()),
            &auth,
        )
        .expect_err("fetch against a repo with no configured remote fails");
        assert!(
            error.contains("install the glab CLI") && error.contains("glab auth login"),
            "expected setup guidance in error, got: {error}"
        );
    }

    #[test]
    fn parse_trusted_external_origins_accepts_bare_https_origins_only() {
        assert_eq!(
            parse_trusted_external_origins("https://gitlab.onlyarag.com"),
            vec!["https://gitlab.onlyarag.com".to_string()],
        );
        // Trailing slash, mixed case host, and a redundant default port all
        // normalize to the same bare origin.
        assert_eq!(
            parse_trusted_external_origins("https://GitLab.OnlyArag.com/"),
            vec!["https://gitlab.onlyarag.com".to_string()],
        );
        assert_eq!(
            parse_trusted_external_origins("https://gitlab.onlyarag.com:443"),
            vec!["https://gitlab.onlyarag.com".to_string()],
        );
        // An explicit non-default port stays significant.
        assert_eq!(
            parse_trusted_external_origins("https://gitlab.onlyarag.com:8443"),
            vec!["https://gitlab.onlyarag.com:8443".to_string()],
        );
        // Multiple entries, whitespace-tolerant.
        assert_eq!(
            parse_trusted_external_origins(" https://a.example , https://b.example "),
            vec![
                "https://a.example".to_string(),
                "https://b.example".to_string()
            ],
        );
    }

    #[test]
    fn parse_trusted_external_origins_drops_invalid_entries() {
        // http (not https), a path beyond `/`, userinfo, query, and
        // fragment are all invalid — dropped, not fatal for the rest of the
        // list.
        assert_eq!(
            parse_trusted_external_origins(
                "http://gitlab.onlyarag.com,\
                 https://gitlab.onlyarag.com/group,\
                 https://user@gitlab.onlyarag.com,\
                 https://gitlab.onlyarag.com?x=1,\
                 https://gitlab.onlyarag.com#frag,\
                 not a url,\
                 ,\
                 https://good.example"
            ),
            vec!["https://good.example".to_string()],
        );
    }

    #[test]
    fn external_https_remote_rejects_hosts_outside_the_trusted_list() {
        let trusted = ["https://gitlab.onlyarag.com".to_string()];
        // gitlab.com is not on the trusted list, so it is rejected outright
        // rather than silently treated as a Buzz repo.
        assert!(
            external_https_remote_with_trusted("https://gitlab.com/block/buzz", &trusted).is_err()
        );
        // A lookalike host is not the trusted origin.
        assert!(external_https_remote_with_trusted(
            "https://gitlab.onlyarag.com.evil.test/block/buzz",
            &trusted
        )
        .is_err());
        // Trusted host, but the wrong port — no implicit match.
        assert!(external_https_remote_with_trusted(
            "https://gitlab.onlyarag.com:8443/block/buzz",
            &trusted
        )
        .is_err());
        // Configuring the exact port allows it.
        let trusted_with_port = ["https://gitlab.onlyarag.com:8443".to_string()];
        assert!(external_https_remote_with_trusted(
            "https://gitlab.onlyarag.com:8443/block/buzz",
            &trusted_with_port
        )
        .unwrap()
        .is_some());
        // github.com is always trusted, independent of the configured list.
        assert!(
            external_https_remote_with_trusted("https://github.com/block/buzz", &[])
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn external_https_remote_filters_trailing_slash_empty_segment() {
        let trusted = ["https://gitlab.onlyarag.com".to_string()];
        // A trailing slash must not count as an extra path segment.
        let remote = external_https_remote_with_trusted("https://github.com/block/buzz/", &trusted)
            .unwrap()
            .expect("trailing slash still names owner/repo");
        assert_eq!(remote.host, "github.com");
        // An empty segment from a doubled slash elsewhere in the path is
        // still rejected.
        assert!(external_https_remote_with_trusted(
            "https://gitlab.onlyarag.com/group//repo",
            &trusted
        )
        .is_err());
    }

    #[test]
    fn trusted_external_origins_env_reads_the_configured_variable() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV).ok();

        std::env::set_var(
            TRUSTED_EXTERNAL_GIT_ORIGINS_ENV,
            "https://gitlab.onlyarag.com, not a url, https://gitlab.com",
        );
        assert_eq!(
            trusted_external_origins_env(),
            vec![
                "https://gitlab.onlyarag.com".to_string(),
                "https://gitlab.com".to_string(),
            ],
        );
        assert!(
            external_https_remote("https://gitlab.onlyarag.com/group/repo")
                .unwrap()
                .is_some()
        );
        assert!(external_https_remote("https://not-configured.example/group/repo").is_err());

        std::env::remove_var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV);
        assert_eq!(trusted_external_origins_env(), Vec::<String>::new());
        assert!(external_https_remote("https://gitlab.onlyarag.com/group/repo").is_err());

        match previous {
            Some(value) => std::env::set_var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV, value),
            None => std::env::remove_var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV),
        }
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
    fn local_clone_url_allows_safe_external_https_urls() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV).ok();
        std::env::remove_var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV);

        assert!(validate_local_clone_url("https://github.com/block/buzz").is_ok());
        assert!(validate_local_clone_url("https://github.com/block/buzz.git").is_ok());
        assert!(validate_local_clone_url("https://github.com:8443/block/buzz").is_err());
        assert!(validate_local_clone_url("http://github.com/block/buzz").is_err());
        assert!(validate_local_clone_url("https://github.com/block/buzz/issues").is_err());
        assert!(validate_local_clone_url("https://user@github.com/block/buzz").is_err());
        assert!(validate_local_clone_url("https://github.com/block/../buzz").is_err());
        assert!(validate_local_clone_url("https://github.com/-upload-pack/buzz").is_err());
        assert!(validate_local_clone_url("ssh://git@github.com/block/buzz").is_err());
        // With no operator-configured trust entry, any external host other
        // than the github.com built-in is rejected outright — this is the
        // fix for the regression that had accepted every https host.
        assert!(
            validate_local_clone_url("https://gitlab.onlyarag.com/group/private-repo.git").is_err()
        );
        assert!(validate_local_clone_url("https://gitlab.com/block/buzz").is_err());
        assert!(validate_local_clone_url("https://github.com.evil.test/block/buzz").is_err());

        match previous {
            Some(value) => std::env::set_var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV, value),
            None => std::env::remove_var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV),
        }
    }

    #[test]
    fn local_clone_url_allows_operator_trusted_external_origin() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV).ok();
        std::env::set_var(
            TRUSTED_EXTERNAL_GIT_ORIGINS_ENV,
            "https://gitlab.onlyarag.com",
        );

        assert!(
            validate_local_clone_url("https://gitlab.onlyarag.com/group/private-repo.git").is_ok()
        );
        assert!(validate_local_clone_url(
            "https://gitlab.onlyarag.com/group/subgroup/private-repo.git"
        )
        .is_ok());
        assert!(
            validate_local_clone_url("https://gitlab.onlyarag.com/group/buzz?token=secret")
                .is_err()
        );
        assert!(validate_local_clone_url("https://gitlab.onlyarag.com/group/buzz#branch").is_err());
        // Still not github.com or the trusted origin.
        assert!(validate_local_clone_url("https://gitlab.com/block/buzz").is_err());

        match previous {
            Some(value) => std::env::set_var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV, value),
            None => std::env::remove_var(TRUSTED_EXTERNAL_GIT_ORIGINS_ENV),
        }
    }
}
