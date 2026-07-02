//! Shared git subprocess plumbing for the project commands.
//!
//! Runs the system `git` with an ephemeral, env-only auth configuration:
//! the identity nsec is handed to `git-credential-nostr` via environment
//! variables so nothing key-related ever touches disk or global git config.

use crate::{app_state::AppState, managed_agents::resolve_command};
use nostr::ToBech32;
use std::process::Command;
use url::Url;

pub(crate) struct GitAuthConfig {
    git_path: std::path::PathBuf,
    credential_helper: Option<std::path::PathBuf>,
    nsec: String,
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
    configure_git_auth(&mut command, auth);
    let output = command
        .output()
        .map_err(|error| format!("failed to run git: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("git exited with status {}", output.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn configure_git_auth(command: &mut Command, auth: &GitAuthConfig) {
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env("GIT_CONFIG_GLOBAL", "/dev/null");
    command.env("GIT_CONFIG_COUNT", "1");
    command.env("GIT_CONFIG_KEY_0", "credential.helper");
    command.env("GIT_CONFIG_VALUE_0", "");
    let Some(cred_helper) = &auth.credential_helper else {
        return;
    };
    command.env("NOSTR_PRIVATE_KEY", &auth.nsec);
    command.env("GIT_CONFIG_COUNT", "3");
    command.env("GIT_CONFIG_KEY_1", "credential.helper");
    command.env("GIT_CONFIG_VALUE_1", cred_helper.display().to_string());
    command.env("GIT_CONFIG_KEY_2", "credential.useHttpPath");
    command.env("GIT_CONFIG_VALUE_2", "true");
}

pub(crate) fn build_git_auth_config(state: &AppState) -> Result<GitAuthConfig, String> {
    let git_path = resolve_command("git").ok_or_else(|| "git was not found on PATH".to_string())?;
    let credential_helper = resolve_command("git-credential-nostr");
    let nsec = {
        let keys = state.keys.lock().map_err(|error| error.to_string())?;
        keys.secret_key()
            .to_bech32()
            .map_err(|error| format!("encode identity key: {error}"))?
    };
    Ok(GitAuthConfig {
        git_path,
        credential_helper,
        nsec,
    })
}

pub(crate) fn validate_clone_url(clone_url: &str) -> Result<(), String> {
    let parsed = Url::parse(clone_url).map_err(|error| format!("invalid clone URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("clone URL must be http or https".into());
    }
    if !parsed.path().contains("/git/") {
        return Err("clone URL must point at a Buzz git repository".into());
    }
    Ok(())
}
