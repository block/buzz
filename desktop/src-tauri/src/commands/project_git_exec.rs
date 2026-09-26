//! Shared git subprocess plumbing for the project commands.
//!
//! Runs the system `git` with an ephemeral, env-only auth configuration:
//! the identity nsec is handed to `git-credential-nostr` via environment
//! variables so nothing key-related ever touches disk or global git config.

use crate::{
    app_state::AppState,
    managed_agents::{login_shell_path, resolve_command},
};
use nostr::{Keys, ToBech32};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use url::Url;

/// Wall-clock cap for a single git invocation. Remote operations talk to
/// relay-supplied clone URLs, so a slow or adversarial remote must not pin
/// `spawn_blocking` threads indefinitely.
const LOCAL_GIT_TIMEOUT: Duration = Duration::from_secs(60);
const REMOTE_GIT_TIMEOUT: Duration = Duration::from_secs(300);
const MIN_GIT_VERSION: (u64, u64) = (2, 46);
/// Deadline for a single `git --version` probe. Discovery runs on the
/// caller's thread before any git work reaches `spawn_blocking`, so a broken
/// candidate must fail fast instead of pinning the first auth-config build.
const PROBE_GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// `git --version` prints one short line; cap the capture so an arbitrary
/// binary on a candidate path cannot balloon memory.
const PROBE_OUTPUT_LIMIT: u64 = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
struct GitVersion {
    major: u64,
    minor: u64,
    display: String,
}

fn parse_git_version(output: &str) -> Option<GitVersion> {
    let display = output.trim().strip_prefix("git version ")?.to_string();
    let numeric = display.split_whitespace().next()?;
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some(GitVersion {
        major,
        minor,
        display,
    })
}

fn probe_git_version(path: &Path) -> Result<GitVersion, String> {
    probe_git_version_with_deadline(path, PROBE_GIT_TIMEOUT)
}

fn probe_git_version_with_deadline(path: &Path, timeout: Duration) -> Result<GitVersion, String> {
    // Capture stdout in a regular file rather than a pipe: a file read never
    // blocks on EOF, so a candidate that leaves a descendant holding the
    // write end cannot stall discovery past the deadline.
    let mut stdout_file =
        tempfile::tempfile().map_err(|error| format!("{}: {error}", path.display()))?;
    let stdout_handle = stdout_file
        .try_clone()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let mut command = Command::new(path);
    command
        .arg("--version")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_handle))
        .stderr(Stdio::null());
    crate::util::configure_no_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("{}: {error}", path.display()))?;

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{}: version probe timed out after {:?}",
                        path.display(),
                        timeout
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{}: failed to wait for version probe: {error}",
                    path.display()
                ));
            }
        }
    };
    if !status.success() {
        return Err(format!("{}: exited with {status}", path.display()));
    }
    stdout_file
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    let _ = stdout_file
        .by_ref()
        .take(PROBE_OUTPUT_LIMIT)
        .read_to_end(&mut bytes);
    let stdout = String::from_utf8_lossy(&bytes);
    parse_git_version(&stdout).ok_or_else(|| {
        format!(
            "{}: unrecognized version output `{}`",
            path.display(),
            stdout.trim()
        )
    })
}

fn select_compatible_git(
    candidates: Vec<PathBuf>,
    mut probe: impl FnMut(&Path) -> Result<GitVersion, String>,
) -> Result<PathBuf, String> {
    if candidates.is_empty() {
        return Err(
            "git was not found on PATH, the login-shell PATH, or the platform install locations"
                .to_string(),
        );
    }

    let mut found = Vec::new();
    for path in candidates {
        match probe(&path) {
            Ok(version) if (version.major, version.minor) >= MIN_GIT_VERSION => {
                return Ok(path);
            }
            Ok(version) => found.push(format!("{} at {}", version.display, path.display())),
            Err(error) => found.push(error),
        }
    }

    Err(format!(
        "Buzz Git authentication requires Git {}.{} or newer. Found: {}. Install a compatible Git and restart Buzz.",
        MIN_GIT_VERSION.0,
        MIN_GIT_VERSION.1,
        found.join(", ")
    ))
}

fn git_candidate_paths() -> Vec<PathBuf> {
    let executable = if cfg!(windows) { "git.exe" } else { "git" };
    let mut candidates = Vec::new();
    let mut push_candidate = |candidate: PathBuf| {
        if candidate.is_file() && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };

    if let Some(path) = resolve_command("git") {
        push_candidate(path);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            push_candidate(directory.join(executable));
        }
    }
    if let Some(path) = login_shell_path() {
        for directory in std::env::split_paths(&path) {
            push_candidate(directory.join(executable));
        }
    }

    #[cfg(not(windows))]
    for path in [
        "/opt/homebrew/bin/git",
        "/opt/local/bin/git",
        "/usr/local/bin/git",
        "/usr/bin/git",
    ] {
        push_candidate(PathBuf::from(path));
    }
    #[cfg(windows)]
    for root in [
        std::env::var_os("ProgramFiles"),
        std::env::var_os("ProgramFiles(x86)"),
    ]
    .into_iter()
    .flatten()
    {
        push_candidate(PathBuf::from(root).join("Git").join("cmd").join(executable));
    }

    candidates
}

fn resolve_compatible_git() -> Result<PathBuf, String> {
    use std::sync::OnceLock;
    static COMPATIBLE_GIT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    COMPATIBLE_GIT
        .get_or_init(|| select_compatible_git(git_candidate_paths(), probe_git_version))
        .clone()
}

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
        let Some(cred_helper) = &auth.credential_helper else {
            return apply_git_config(command, &entries);
        };
        command.env("NOSTR_PRIVATE_KEY", &auth.nsec);
        entries.push((
            "credential.helper",
            credential_helper_config_value(cred_helper),
        ));
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

pub(crate) fn build_git_clone_auth_config(
    clone_url: &str,
    state: &AppState,
) -> Result<GitAuthConfig, String> {
    if validate_github_clone_url(clone_url).is_ok() {
        return Ok(GitAuthConfig {
            git_path: resolve_command("git")
                .ok_or_else(|| "git was not found on PATH".to_string())?,
            credential_helper: None,
            nsec: String::new(),
            allow_file_transport: false,
        });
    }
    build_git_auth_config(state)
}

pub(crate) fn build_git_auth_config_for_keys(keys: &Keys) -> Result<GitAuthConfig, String> {
    let git_path = resolve_compatible_git()?;
    let credential_helper = resolve_command("git-credential-nostr");
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

fn validate_github_clone_url(clone_url: &str) -> Result<(), String> {
    let parsed = Url::parse(clone_url).map_err(|error| format!("invalid clone URL: {error}"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("GitHub clone URL must use public https://github.com/owner/repository".into());
    }
    let segments = parsed
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let valid_segment = |segment: &&str| {
        !segment.starts_with('-')
            && !segment.contains("..")
            && segment.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            })
    };
    if segments.len() != 2 || !segments.iter().all(valid_segment) {
        return Err("GitHub clone URL must name one owner and repository".into());
    }
    Ok(())
}

pub(crate) fn validate_local_clone_url(clone_url: &str) -> Result<(), String> {
    if validate_clone_url(clone_url).is_ok() || validate_github_clone_url(clone_url).is_ok() {
        return Ok(());
    }
    Err("clone URL must point at a Buzz repository or public GitHub repository".into())
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
        clean_branch, clean_target_ref, credential_helper_config_value, git_needs_credentials,
        git_subcommand, parse_git_version, select_compatible_git, validate_clone_url,
        validate_clone_url_against_relay, validate_local_clone_url, GitVersion,
    };

    #[test]
    fn parses_platform_git_versions() {
        assert_eq!(
            parse_git_version("git version 2.39.5 (Apple Git-154)\n"),
            Some(GitVersion {
                major: 2,
                minor: 39,
                display: "2.39.5 (Apple Git-154)".to_string(),
            })
        );
        assert_eq!(
            parse_git_version("git version 2.47.1.windows.1\n"),
            Some(GitVersion {
                major: 2,
                minor: 47,
                display: "2.47.1.windows.1".to_string(),
            })
        );
    }

    #[test]
    fn compatible_git_selection_skips_an_older_first_candidate() {
        let old = std::path::PathBuf::from("/usr/bin/git");
        let current = std::path::PathBuf::from("/opt/local/bin/git");
        let selected = select_compatible_git(vec![old.clone(), current.clone()], |path| {
            if path == old {
                Ok(GitVersion {
                    major: 2,
                    minor: 39,
                    display: "2.39.5 (Apple Git-154)".to_string(),
                })
            } else {
                Ok(GitVersion {
                    major: 2,
                    minor: 47,
                    display: "2.47.0".to_string(),
                })
            }
        })
        .expect("compatible Git should be selected");

        assert_eq!(selected, current);
    }

    #[cfg(unix)]
    #[test]
    fn version_probe_kills_a_hung_git_and_returns_promptly() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("create temp dir");
        let fake_git = dir.path().join("git");
        let mut file = std::fs::File::create(&fake_git).expect("create fake git");
        write!(file, "#!/bin/sh\nsleep 600\n").expect("write fake git");
        drop(file);
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755))
            .expect("mark fake git executable");

        let started = std::time::Instant::now();
        let error = super::probe_git_version_with_deadline(
            &fake_git,
            std::time::Duration::from_millis(200),
        )
        .expect_err("hung probe must time out");

        assert!(error.contains("timed out"), "unexpected error: {error}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "probe did not return promptly: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn empty_candidate_error_reports_all_searched_locations() {
        let error = select_compatible_git(vec![], |_| -> Result<GitVersion, String> {
            unreachable!("no candidates to probe")
        })
        .expect_err("empty candidates must error");
        assert!(
            error.contains("login-shell PATH"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn incompatible_git_error_names_the_version_and_path() {
        let path = std::path::PathBuf::from("/usr/bin/git");
        let error = select_compatible_git(vec![path], |_| {
            Ok(GitVersion {
                major: 2,
                minor: 39,
                display: "2.39.5 (Apple Git-154)".to_string(),
            })
        })
        .expect_err("old Git must be rejected");

        assert!(error.contains("requires Git 2.46 or newer"));
        assert!(error.contains("2.39.5 (Apple Git-154) at /usr/bin/git"));
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
    fn local_clone_url_allows_only_public_github_https_urls() {
        assert!(validate_local_clone_url("https://github.com/block/buzz").is_ok());
        assert!(validate_local_clone_url("https://github.com/block/buzz.git").is_ok());
        assert!(validate_local_clone_url("http://github.com/block/buzz").is_err());
        assert!(validate_local_clone_url("https://github.com/block/buzz/issues").is_err());
        assert!(validate_local_clone_url("https://user@github.com/block/buzz").is_err());
        assert!(validate_local_clone_url("https://github.com.evil.test/block/buzz").is_err());
        assert!(validate_local_clone_url("https://gitlab.com/block/buzz").is_err());
    }
}
