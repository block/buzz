use std::process::Command;

use nostr::ToBech32;
use serde::Serialize;
use tauri::State;
use url::Url;

use crate::{app_state::AppState, managed_agents::resolve_command};

#[derive(Serialize)]
pub struct ProjectRepoCommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
    pub subject: String,
}

#[derive(Serialize)]
pub struct ProjectRepoFileInfo {
    pub path: String,
    pub kind: String,
    pub size: Option<u64>,
    pub preview_content: Option<String>,
}

#[derive(Serialize)]
pub struct ProjectRepoSnapshotInfo {
    pub latest_commit: Option<ProjectRepoCommitInfo>,
    pub files: Vec<ProjectRepoFileInfo>,
}

struct GitAuthConfig {
    git_path: std::path::PathBuf,
    credential_helper: Option<std::path::PathBuf>,
    nsec: String,
}

fn run_git(
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

    // Clear inherited/global helpers first. Otherwise Git may authenticate with
    // git-credential-nostr successfully, then call a system helper such as
    // git-credential-osxkeychain to `store` the ephemeral NIP-98 credential;
    // that helper can fail and make an otherwise-successful read look broken.
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

fn build_git_auth_config(state: &AppState) -> Result<GitAuthConfig, String> {
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

fn validate_clone_url(clone_url: &str) -> Result<(), String> {
    let parsed = Url::parse(clone_url).map_err(|error| format!("invalid clone URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("clone URL must be http or https".into());
    }
    if !parsed.path().contains("/git/") {
        return Err("clone URL must point at a Buzz git repository".into());
    }
    Ok(())
}

fn parse_latest_commit(output: &str) -> Option<ProjectRepoCommitInfo> {
    let line = output.lines().next()?;
    let mut parts = line.split('\0');
    let hash = parts.next()?.to_string();
    let short_hash = parts.next()?.to_string();
    let author_name = parts.next()?.to_string();
    let author_email = parts.next()?.to_string();
    let timestamp = parts.next()?.parse::<i64>().ok()?;
    let subject = parts.next().unwrap_or_default().to_string();

    Some(ProjectRepoCommitInfo {
        hash,
        short_hash,
        author_name,
        author_email,
        timestamp,
        subject,
    })
}

fn read_preview_content(
    repo_dir: &std::path::Path,
    path: &str,
    size: Option<u64>,
) -> Option<String> {
    const MAX_PREVIEW_BYTES: u64 = 64 * 1024;
    if size.is_some_and(|value| value > MAX_PREVIEW_BYTES) {
        return None;
    }

    let full_path = repo_dir.join(path);
    let normalized = full_path.canonicalize().ok()?;
    let repo_root = repo_dir.canonicalize().ok()?;
    if !normalized.starts_with(repo_root) {
        return None;
    }

    let bytes = std::fs::read(normalized).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn parse_ls_tree(repo_dir: &std::path::Path, output: &str) -> Vec<ProjectRepoFileInfo> {
    output
        .lines()
        .filter_map(|line| {
            let (meta, path) = line.split_once('\t')?;
            let mut parts = meta.split_whitespace();
            let _mode = parts.next()?;
            let kind = parts.next()?.to_string();
            let _object = parts.next()?;
            let size = parts.next().and_then(|value| value.parse::<u64>().ok());
            let preview_content = if kind == "blob" {
                read_preview_content(repo_dir, path, size)
            } else {
                None
            };
            Some(ProjectRepoFileInfo {
                path: path.to_string(),
                kind,
                size,
                preview_content,
            })
        })
        .take(250)
        .collect()
}

fn snapshot_from_repo(repo_dir: &std::path::Path, auth: &GitAuthConfig) -> ProjectRepoSnapshotInfo {
    let latest_commit = run_git(
        &["log", "-1", "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%s"],
        Some(repo_dir),
        auth,
    )
    .ok()
    .and_then(|output| parse_latest_commit(&output));
    let files = if latest_commit.is_some() {
        run_git(&["ls-tree", "-r", "--long", "HEAD"], Some(repo_dir), auth)
            .map(|output| parse_ls_tree(repo_dir, &output))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    ProjectRepoSnapshotInfo {
        latest_commit,
        files,
    }
}

fn current_checkout_snapshot(auth: &GitAuthConfig) -> Option<ProjectRepoSnapshotInfo> {
    if !cfg!(debug_assertions) {
        return None;
    }

    let cwd = std::env::current_dir().ok()?;
    let root = run_git(&["rev-parse", "--show-toplevel"], Some(&cwd), auth).ok()?;
    let root = std::path::PathBuf::from(root.trim());
    Some(snapshot_from_repo(&root, auth))
}

#[tauri::command]
pub async fn get_project_repo_snapshot(
    clone_url: String,
    default_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoSnapshotInfo, String> {
    validate_clone_url(&clone_url)?;
    let auth = build_git_auth_config(&state)?;
    let branch = default_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    tauri::async_runtime::spawn_blocking(move || {
        let temp_dir = tempfile::tempdir().map_err(|error| format!("create temp dir: {error}"))?;
        let repo_dir = temp_dir.path().join("repo");
        let repo_path = repo_dir
            .to_str()
            .ok_or_else(|| "temporary repository path is not UTF-8".to_string())?;

        let mut clone_args = vec!["clone", "--depth=1", "--filter=blob:none"];
        if let Some(ref branch) = branch {
            clone_args.push("--branch");
            clone_args.push(branch.as_str());
        }
        clone_args.push(clone_url.as_str());
        clone_args.push(repo_path);

        if run_git(&clone_args, None, &auth).is_err() && branch.is_some() {
            // A newly-announced Buzz git repo can be empty even when the project
            // metadata says its default branch is `main`. In that state,
            // `git clone --branch main` fails because the ref does not exist yet.
            // Retry without the branch selector so we can still render a clean
            // "no commits/files yet" state instead of an error card.
            run_git(
                &[
                    "clone",
                    "--depth=1",
                    "--filter=blob:none",
                    clone_url.as_str(),
                    repo_path,
                ],
                None,
                &auth,
            )?;
        }

        let snapshot = snapshot_from_repo(&repo_dir, &auth);
        if snapshot.latest_commit.is_none() && snapshot.files.is_empty() {
            if let Some(local_snapshot) = current_checkout_snapshot(&auth) {
                return Ok(local_snapshot);
            }
        }

        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("repo snapshot task failed: {error}"))?
}
