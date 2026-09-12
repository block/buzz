use crate::{client::BuzzClient, error::CliError};
use nostr::ToBech32;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use url::Url;

const GIT_READBACK_TIMEOUT: Duration = Duration::from_secs(60);

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn create() -> Result<Self, CliError> {
        let path = std::env::temp_dir().join(format!("buzz-bw-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path)
            .map_err(|error| CliError::Other(format!("create Git readback directory: {error}")))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

pub(super) fn validate_git_commit(commit: &str) -> Result<String, CliError> {
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::Usage(
            "commit must be a 40-character Git SHA-1".into(),
        ));
    }
    Ok(commit.to_ascii_lowercase())
}

fn validate_stream_for_git(stream: &str) -> Result<(), CliError> {
    let bytes = stream.as_bytes();
    let valid = !stream.is_empty()
        && stream.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'/' | b'-')
        })
        && !stream.contains("..")
        && !stream.contains("@{")
        && !stream.ends_with(['.', '/'])
        && stream
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"));
    if valid {
        Ok(())
    } else {
        Err(CliError::Usage("invalid BW stream name".into()))
    }
}

fn tag<'a>(event: &'a Value, name: &str) -> Option<&'a str> {
    event["tags"]
        .as_array()?
        .iter()
        .find(|tag| tag[0] == name)?[1]
        .as_str()
}

/// Resolve the clone URL only from the exact genesis that Core authenticated,
/// then bind it to this CLI's active relay and requested repository. The URL
/// is public signed metadata, but it must not redirect a process carrying the
/// writer identity to another origin or repository.
pub(super) fn genesis_clone_url(
    events: &BTreeMap<String, Value>,
    genesis_id: &str,
    relay_url: &str,
    repo_owner: &str,
    repo_id: &str,
) -> Result<String, CliError> {
    let event = events
        .get(&genesis_id.to_ascii_lowercase())
        .ok_or_else(|| CliError::Other("authenticated BW genesis is missing".into()))?;
    let clone_url = tag(event, "clone")
        .ok_or_else(|| CliError::Usage("repository announcement has no clone URL".into()))?;
    let clone = Url::parse(clone_url)
        .map_err(|error| CliError::Usage(format!("invalid repository clone URL: {error}")))?;
    let relay = Url::parse(&crate::client::normalize_relay_url(relay_url))
        .map_err(|error| CliError::Other(format!("invalid active relay URL: {error}")))?;
    if !matches!(clone.scheme(), "http" | "https")
        || clone.scheme() != relay.scheme()
        || clone.host_str() != relay.host_str()
        || clone.port_or_known_default() != relay.port_or_known_default()
        || !clone.username().is_empty()
        || clone.password().is_some()
        || clone.query().is_some()
        || clone.fragment().is_some()
    {
        return Err(CliError::Usage(
            "repository clone URL must use the active relay without credentials or parameters"
                .into(),
        ));
    }
    let relay_path = relay.path().trim_end_matches('/');
    let expected_path = format!(
        "{relay_path}/git/{}/{}",
        repo_owner.to_ascii_lowercase(),
        repo_id
    );
    if clone.path() != expected_path {
        return Err(CliError::Usage(
            "repository clone URL does not match the active BW repository".into(),
        ));
    }
    Ok(clone_url.to_owned())
}

fn read_pipe_lossy(pipe: Option<impl Read>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).to_string()
}

pub(super) fn resolve_stream_head_blocking(
    clone_url: &str,
    stream: &str,
    client: &BuzzClient,
) -> Result<String, CliError> {
    validate_stream_for_git(stream)?;
    let scratch = ScratchDirectory::create()?;
    let refname = format!("refs/heads/{stream}");
    let nsec = client
        .keys()
        .secret_key()
        .to_bech32()
        .map_err(|error| CliError::Other(format!("encode Git readback identity: {error}")))?;
    let mut command = Command::new("git");
    command
        .args([
            "ls-remote",
            "--exit-code",
            "--end-of-options",
            clone_url,
            refname.as_str(),
        ])
        .current_dir(scratch.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CEILING_DIRECTORIES", scratch.path())
        .env("NOSTR_PRIVATE_KEY", nsec)
        .env_remove("BUZZ_PRIVATE_KEY");
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
    let entries = [
        ("credential.helper", ""),
        ("credential.helper", "nostr"),
        ("credential.useHttpPath", "true"),
        ("core.hooksPath", "/dev/null"),
        ("core.fsmonitor", "false"),
        ("protocol.allow", "never"),
        ("protocol.http.allow", "always"),
        ("protocol.https.allow", "always"),
        ("protocol.ext.allow", "never"),
        ("protocol.file.allow", "never"),
    ];
    command.env("GIT_CONFIG_COUNT", entries.len().to_string());
    for (index, (key, value)) in entries.iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    if let Some(auth_tag) = client.bw_auth_tag() {
        let auth_tag = serde_json::to_string(auth_tag.as_slice())
            .map_err(|error| CliError::Other(format!("encode Git readback auth: {error}")))?;
        command.env("BUZZ_AUTH_TAG", auth_tag);
    } else {
        command.env_remove("BUZZ_AUTH_TAG");
    }
    let mut child = command
        .spawn()
        .map_err(|error| CliError::Other(format!("start Git readback: {error}")))?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || read_pipe_lossy(stdout_pipe));
    let stderr_thread = std::thread::spawn(move || read_pipe_lossy(stderr_pipe));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() <= GIT_READBACK_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(CliError::Other(format!(
                    "Git readback timed out after {} seconds",
                    GIT_READBACK_TIMEOUT.as_secs()
                )));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(CliError::Other(format!("wait for Git readback: {error}")));
            }
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        let stderr = stderr.trim();
        return Err(CliError::Other(if stderr.is_empty() {
            format!("Git readback exited with status {status}")
        } else {
            format!("Git readback failed: {stderr}")
        }));
    }
    parse_stream_head(&stdout, &refname)
}

fn parse_stream_head(output: &str, refname: &str) -> Result<String, CliError> {
    output
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let sha = parts.next()?;
            let matched_ref = parts.next()?;
            (matched_ref == refname).then(|| sha.to_ascii_lowercase())
        })
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| CliError::Other(format!("could not resolve remote head for {refname}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn readback_uses_only_the_exact_branch_ref() {
        let real = "1".repeat(40);
        let shadow = "2".repeat(40);
        let output = format!(
            "{shadow}\trefs/heads/x/refs/heads/windows-integration\n{real}\trefs/heads/windows-integration\n"
        );
        assert_eq!(
            parse_stream_head(&output, "refs/heads/windows-integration").expect("real head"),
            real
        );
        assert!(parse_stream_head(
            &format!("{shadow}\trefs/heads/x/refs/heads/windows-integration\n"),
            "refs/heads/windows-integration"
        )
        .is_err());
    }

    #[test]
    fn clone_url_is_bound_to_the_active_repo() {
        let owner = "a".repeat(64);
        let genesis = "b".repeat(64);
        let mut events = BTreeMap::new();
        events.insert(
            genesis,
            json!({
                "id": "b".repeat(64),
                "tags": [["clone", format!("https://relay.example/base/git/{owner}/buzz")]],
            }),
        );
        assert_eq!(
            genesis_clone_url(
                &events,
                &"b".repeat(64),
                "wss://relay.example/base",
                &owner,
                "buzz"
            )
            .expect("same relay repository"),
            format!("https://relay.example/base/git/{owner}/buzz")
        );
        assert!(genesis_clone_url(
            &events,
            &"b".repeat(64),
            "wss://relay.example/base",
            &owner,
            "other"
        )
        .is_err());
        assert!(genesis_clone_url(
            &events,
            &"b".repeat(64),
            "wss://evil.example/base",
            &owner,
            "buzz"
        )
        .is_err());
    }
}
