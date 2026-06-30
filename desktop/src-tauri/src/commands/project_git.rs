use std::process::Command;
use std::time::UNIX_EPOCH;

use nostr::ToBech32;
use serde::Serialize;
use tauri::State;
use url::Url;

use crate::{
    app_state::AppState,
    managed_agents::{nest_dir, resolve_command},
};

#[derive(Clone, Serialize)]
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
    pub last_changed_at: Option<i64>,
    pub latest_commit: Option<ProjectRepoCommitInfo>,
}

#[derive(Serialize)]
pub struct ProjectRepoContributorInfo {
    pub name: String,
    pub email: String,
    pub commit_count: usize,
    pub last_commit_at: i64,
}

#[derive(Serialize)]
pub struct ProjectRepoSnapshotInfo {
    pub latest_commit: Option<ProjectRepoCommitInfo>,
    pub commits: Vec<ProjectRepoCommitInfo>,
    pub files: Vec<ProjectRepoFileInfo>,
    pub contributors: Vec<ProjectRepoContributorInfo>,
}

#[derive(Serialize)]
pub struct ProjectLocalRepoSnapshotInfo {
    pub path: String,
    pub snapshot: ProjectRepoSnapshotInfo,
}

#[derive(Serialize)]
pub struct ProjectLocalRepoInfo {
    pub name: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct ProjectRepoSyncStatusInfo {
    pub local_path: Option<String>,
    pub local_branch: Option<String>,
    pub local_head: Option<String>,
    pub local_short_head: Option<String>,
    pub remote_branch: Option<String>,
    pub remote_head: Option<String>,
    pub remote_short_head: Option<String>,
    pub ahead_count: usize,
    pub behind_count: usize,
    pub has_uncommitted_changes: bool,
    pub has_untracked_files: bool,
    pub can_push: bool,
    pub push_block_reason: Option<String>,
}

#[derive(Serialize)]
pub struct ProjectRepoPushResult {
    pub pushed: bool,
    pub message: String,
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

fn short_hash(hash: &str) -> String {
    hash.chars().take(7).collect()
}

fn first_output_line(output: &str) -> Option<String> {
    output
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_count(output: &str) -> usize {
    output.trim().parse::<usize>().unwrap_or_default()
}

fn has_uncommitted_changes(output: &str) -> bool {
    output
        .lines()
        .any(|line| !line.starts_with("??") && !line.trim().is_empty())
}

fn has_untracked_files(output: &str) -> bool {
    output.lines().any(|line| line.starts_with("??"))
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

fn parse_commits(output: &str) -> Vec<ProjectRepoCommitInfo> {
    output
        .lines()
        .filter_map(parse_latest_commit)
        .take(50)
        .collect()
}

fn parse_contributors(output: &str) -> Vec<ProjectRepoContributorInfo> {
    let mut contributors: std::collections::HashMap<String, ProjectRepoContributorInfo> =
        std::collections::HashMap::new();

    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.split('\0');
        let name = parts.next().unwrap_or_default().trim().to_string();
        let email = parts.next().unwrap_or_default().trim().to_string();
        let timestamp = parts
            .next()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or_default();
        if name.is_empty() && email.is_empty() {
            continue;
        }

        let key = if email.is_empty() {
            name.to_lowercase()
        } else {
            email.to_lowercase()
        };
        contributors
            .entry(key)
            .and_modify(|contributor| {
                contributor.commit_count += 1;
                contributor.last_commit_at = contributor.last_commit_at.max(timestamp);
            })
            .or_insert(ProjectRepoContributorInfo {
                name,
                email,
                commit_count: 1,
                last_commit_at: timestamp,
            });
    }

    let mut contributors = contributors.into_values().collect::<Vec<_>>();
    contributors.sort_by(|left, right| {
        right
            .commit_count
            .cmp(&left.commit_count)
            .then_with(|| right.last_commit_at.cmp(&left.last_commit_at))
            .then_with(|| left.name.cmp(&right.name))
    });
    contributors.truncate(50);
    contributors
}

fn parse_latest_commit_by_path(
    output: &str,
) -> std::collections::HashMap<String, ProjectRepoCommitInfo> {
    let mut current_commit = None;
    let mut result = std::collections::HashMap::new();

    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        if line.contains('\0') {
            current_commit = parse_latest_commit(line);
            continue;
        }

        if let Some(commit) = &current_commit {
            result
                .entry(line.to_string())
                .or_insert_with(|| commit.clone());
        }
    }

    result
}

fn path_modified_at(path: &std::path::Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}

fn parse_worktree_files(
    repo_dir: &std::path::Path,
    output: &str,
    latest_commit_by_path: &std::collections::HashMap<String, ProjectRepoCommitInfo>,
) -> Vec<ProjectRepoFileInfo> {
    output
        .split('\0')
        .filter(|path| !path.trim().is_empty())
        .filter_map(|path| {
            let full_path = repo_dir.join(path);
            let metadata = std::fs::metadata(&full_path).ok()?;
            if !metadata.is_file() {
                return None;
            }
            let size = Some(metadata.len());
            let latest_commit = latest_commit_by_path.get(path).cloned();
            Some(ProjectRepoFileInfo {
                path: path.to_string(),
                kind: "blob".to_string(),
                size,
                preview_content: read_preview_content(repo_dir, path, size),
                last_changed_at: latest_commit
                    .as_ref()
                    .map(|commit| commit.timestamp)
                    .or_else(|| path_modified_at(&full_path)),
                latest_commit,
            })
        })
        .take(250)
        .collect()
}

fn normalize_branch_name(branch: &str) -> &str {
    branch
        .trim()
        .strip_prefix("refs/heads/")
        .unwrap_or_else(|| branch.trim())
}

fn branch_activity_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch_name: Option<&str>,
    base_branch: Option<&str>,
) -> Option<String> {
    let branch_name = branch_name.map(normalize_branch_name)?;
    let base_branch = base_branch.map(normalize_branch_name)?;

    if branch_name.is_empty() || base_branch.is_empty() || branch_name == base_branch {
        return None;
    }

    let remote_base_ref = format!("refs/remotes/origin/{base_branch}");
    if run_git(
        &["rev-parse", "--verify", "--quiet", remote_base_ref.as_str()],
        Some(repo_dir),
        auth,
    )
    .is_err()
    {
        return None;
    }

    Some(format!("origin/{base_branch}..HEAD"))
}

fn parse_ls_tree(
    repo_dir: &std::path::Path,
    output: &str,
    latest_commit_by_path: &std::collections::HashMap<String, ProjectRepoCommitInfo>,
) -> Vec<ProjectRepoFileInfo> {
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
                last_changed_at: latest_commit_by_path
                    .get(path)
                    .map(|commit| commit.timestamp),
                latest_commit: latest_commit_by_path.get(path).cloned(),
            })
        })
        .take(250)
        .collect()
}

fn snapshot_from_repo(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch_name: Option<&str>,
    base_branch: Option<&str>,
) -> ProjectRepoSnapshotInfo {
    let latest_commit = run_git(
        &["log", "-1", "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%s"],
        Some(repo_dir),
        auth,
    )
    .ok()
    .and_then(|output| parse_latest_commit(&output));
    let branch_activity_range = branch_activity_range(repo_dir, auth, branch_name, base_branch);
    let branch_activity_ref = branch_activity_range.as_deref().unwrap_or("HEAD");
    let (commits, contributors) = if latest_commit.is_some() {
        let commits = run_git(
            &[
                "log",
                "--max-count=50",
                "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%s",
                branch_activity_ref,
            ],
            Some(repo_dir),
            auth,
        )
        .map(|output| parse_commits(&output))
        .unwrap_or_default();
        let contributors = run_git(
            &["log", "--format=%an%x00%ae%x00%at", branch_activity_ref],
            Some(repo_dir),
            auth,
        )
        .map(|output| parse_contributors(&output))
        .unwrap_or_default();
        (commits, contributors)
    } else {
        (Vec::new(), Vec::new())
    };

    let files = if latest_commit.is_some() {
        let latest_commit_by_path = run_git(
            &[
                "log",
                "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%s",
                "--name-only",
                "--diff-filter=ACMRT",
                "--",
            ],
            Some(repo_dir),
            auth,
        )
        .map(|output| parse_latest_commit_by_path(&output))
        .unwrap_or_default();

        run_git(&["ls-tree", "-r", "--long", "HEAD"], Some(repo_dir), auth)
            .map(|output| parse_ls_tree(repo_dir, &output, &latest_commit_by_path))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    ProjectRepoSnapshotInfo {
        latest_commit,
        commits,
        files,
        contributors,
    }
}

fn snapshot_from_worktree(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch_name: Option<&str>,
    base_branch: Option<&str>,
) -> ProjectRepoSnapshotInfo {
    let latest_commit = run_git(
        &["log", "-1", "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%s"],
        Some(repo_dir),
        auth,
    )
    .ok()
    .and_then(|output| parse_latest_commit(&output));
    let branch_activity_range = branch_activity_range(repo_dir, auth, branch_name, base_branch);
    let branch_activity_ref = branch_activity_range.as_deref().unwrap_or("HEAD");
    let (commits, contributors, latest_commit_by_path) = if latest_commit.is_some() {
        let commits = run_git(
            &[
                "log",
                "--max-count=50",
                "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%s",
                branch_activity_ref,
            ],
            Some(repo_dir),
            auth,
        )
        .map(|output| parse_commits(&output))
        .unwrap_or_default();
        let contributors = run_git(
            &["log", "--format=%an%x00%ae%x00%at", branch_activity_ref],
            Some(repo_dir),
            auth,
        )
        .map(|output| parse_contributors(&output))
        .unwrap_or_default();
        let latest_commit_by_path = run_git(
            &[
                "log",
                "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%s",
                "--name-only",
                "--diff-filter=ACMRT",
                "--",
            ],
            Some(repo_dir),
            auth,
        )
        .map(|output| parse_latest_commit_by_path(&output))
        .unwrap_or_default();
        (commits, contributors, latest_commit_by_path)
    } else {
        (Vec::new(), Vec::new(), std::collections::HashMap::new())
    };

    let files = run_git(
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
        Some(repo_dir),
        auth,
    )
    .map(|output| parse_worktree_files(repo_dir, &output, &latest_commit_by_path))
    .unwrap_or_default();

    ProjectRepoSnapshotInfo {
        latest_commit,
        commits,
        files,
        contributors,
    }
}

fn local_repo_name_candidate(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches(".git");
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn clone_url_repo_name(clone_url: &str) -> Option<String> {
    let parsed = Url::parse(clone_url).ok()?;
    let last_segment = parsed
        .path_segments()?
        .filter(|part| !part.is_empty())
        .last()?;
    local_repo_name_candidate(last_segment)
}

fn local_repo_candidates(project_dtag: &str, clone_url: Option<&str>) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(candidate) = local_repo_name_candidate(project_dtag) {
        candidates.push(candidate);
    }
    if let Some(candidate) = clone_url.and_then(clone_url_repo_name) {
        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn find_local_repo_dir(
    repos_dir: Option<&str>,
    project_dtag: &str,
    clone_url: Option<&str>,
) -> Result<Option<std::path::PathBuf>, String> {
    let repos_root = canonical_repos_root(repos_dir)?;

    for candidate in local_repo_candidates(project_dtag, clone_url) {
        let candidate_path = repos_root.join(candidate);
        let Ok(candidate_path) = candidate_path.canonicalize() else {
            continue;
        };
        if !candidate_path.starts_with(&repos_root) || !candidate_path.is_dir() {
            continue;
        }
        if candidate_path.join(".git").exists() {
            return Ok(Some(candidate_path));
        }
    }

    Ok(None)
}

fn canonical_repos_root(repos_dir: Option<&str>) -> Result<std::path::PathBuf, String> {
    let repos_root = repos_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            nest_dir()
                .map(|path| path.join("REPOS"))
                .unwrap_or_else(|| std::path::PathBuf::from("REPOS"))
        });
    if !repos_root.is_absolute() {
        return Err("reposDir must be an absolute path".to_string());
    }
    let repos_root = repos_root
        .canonicalize()
        .map_err(|error| format!("reposDir is not accessible: {error}"))?;
    if !repos_root.is_dir() {
        return Err("reposDir is not a directory".to_string());
    }
    Ok(repos_root)
}

fn normalize_branch_option(branch: Option<&str>) -> Option<String> {
    branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_branch_name)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn compare_local_remote_status(
    repo_dir: &std::path::Path,
    clone_url: &str,
    branch_name: Option<&str>,
    auth: &GitAuthConfig,
) -> ProjectRepoSyncStatusInfo {
    let local_branch = run_git(&["branch", "--show-current"], Some(repo_dir), auth)
        .ok()
        .and_then(|output| first_output_line(&output));
    let branch = normalize_branch_option(branch_name)
        .or_else(|| local_branch.clone())
        .unwrap_or_else(|| "main".to_string());

    let _ = run_git(
        &["remote", "set-url", "origin", clone_url],
        Some(repo_dir),
        auth,
    );
    let _ = run_git(
        &["fetch", "--quiet", "origin", branch.as_str(), "--depth=100"],
        Some(repo_dir),
        auth,
    );

    let local_head = run_git(&["rev-parse", "HEAD"], Some(repo_dir), auth)
        .ok()
        .and_then(|output| first_output_line(&output));
    let remote_ref = format!("refs/remotes/origin/{branch}");
    let remote_head = run_git(
        &["rev-parse", "--verify", "--quiet", remote_ref.as_str()],
        Some(repo_dir),
        auth,
    )
    .ok()
    .and_then(|output| first_output_line(&output));
    let status = run_git(&["status", "--porcelain"], Some(repo_dir), auth).unwrap_or_default();
    let has_uncommitted_changes = has_uncommitted_changes(&status);
    let has_untracked_files = has_untracked_files(&status);
    let ahead_count = match remote_head.as_deref() {
        Some(_) => run_git(
            &[
                "rev-list",
                "--count",
                format!("origin/{branch}..HEAD").as_str(),
            ],
            Some(repo_dir),
            auth,
        )
        .map(|output| parse_count(&output))
        .unwrap_or_default(),
        None => usize::from(local_head.is_some()),
    };
    let behind_count = match remote_head.as_deref() {
        Some(_) => run_git(
            &[
                "rev-list",
                "--count",
                format!("HEAD..origin/{branch}").as_str(),
            ],
            Some(repo_dir),
            auth,
        )
        .map(|output| parse_count(&output))
        .unwrap_or_default(),
        None => 0,
    };

    let push_block_reason = if local_head.is_none() {
        Some("No local commits to push.".to_string())
    } else if has_uncommitted_changes || has_untracked_files {
        Some("Commit or discard local changes before pushing.".to_string())
    } else if behind_count > 0 {
        Some("Pull or reconcile remote commits before pushing.".to_string())
    } else if ahead_count == 0 {
        Some("Local branch is already pushed.".to_string())
    } else {
        None
    };

    ProjectRepoSyncStatusInfo {
        local_path: Some(repo_dir.display().to_string()),
        local_branch,
        local_head: local_head.clone(),
        local_short_head: local_head.as_deref().map(short_hash),
        remote_branch: Some(branch),
        remote_head: remote_head.clone(),
        remote_short_head: remote_head.as_deref().map(short_hash),
        ahead_count,
        behind_count,
        has_uncommitted_changes,
        has_untracked_files,
        can_push: push_block_reason.is_none(),
        push_block_reason,
    }
}

#[tauri::command]
pub async fn get_project_repo_snapshot(
    clone_url: String,
    default_branch: Option<String>,
    base_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoSnapshotInfo, String> {
    validate_clone_url(&clone_url)?;
    let auth = build_git_auth_config(&state)?;
    let branch = default_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let base_branch = base_branch
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

        let mut clone_args = vec!["clone", "--filter=blob:none"];
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
                &["clone", "--filter=blob:none", clone_url.as_str(), repo_path],
                None,
                &auth,
            )?;
        }

        let snapshot =
            snapshot_from_repo(&repo_dir, &auth, branch.as_deref(), base_branch.as_deref());
        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("repo snapshot task failed: {error}"))?
}

#[tauri::command]
pub async fn get_project_local_repo_snapshot(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: Option<String>,
    default_branch: Option<String>,
    base_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<ProjectLocalRepoSnapshotInfo>, String> {
    let auth = build_git_auth_config(&state)?;
    let branch = default_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let base_branch = base_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    tauri::async_runtime::spawn_blocking(move || {
        let Some(repo_dir) =
            find_local_repo_dir(repos_dir.as_deref(), &project_dtag, clone_url.as_deref())?
        else {
            return Ok(None);
        };
        let snapshot =
            snapshot_from_worktree(&repo_dir, &auth, branch.as_deref(), base_branch.as_deref());
        Ok(Some(ProjectLocalRepoSnapshotInfo {
            path: repo_dir.display().to_string(),
            snapshot,
        }))
    })
    .await
    .map_err(|error| format!("local repo snapshot task failed: {error}"))?
}

#[tauri::command]
pub async fn list_project_local_repositories(
    repos_dir: Option<String>,
) -> Result<Vec<ProjectLocalRepoInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repos_root = canonical_repos_root(repos_dir.as_deref())?;
        let entries =
            std::fs::read_dir(&repos_root).map_err(|error| format!("read reposDir: {error}"))?;
        let mut repos = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                if !file_type.is_dir() && !file_type.is_symlink() {
                    return None;
                }
                let path = entry.path().canonicalize().ok()?;
                if !path.starts_with(&repos_root) || !path.is_dir() || !path.join(".git").exists() {
                    return None;
                }
                Some(ProjectLocalRepoInfo {
                    name: entry.file_name().to_string_lossy().to_string(),
                    path: path.display().to_string(),
                })
            })
            .collect::<Vec<_>>();
        repos.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(repos)
    })
    .await
    .map_err(|error| format!("local repo list task failed: {error}"))?
}

#[tauri::command]
pub async fn get_project_repo_sync_status(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: String,
    default_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoSyncStatusInfo, String> {
    validate_clone_url(&clone_url)?;
    let auth = build_git_auth_config(&state)?;

    tauri::async_runtime::spawn_blocking(move || {
        let Some(repo_dir) =
            find_local_repo_dir(repos_dir.as_deref(), &project_dtag, Some(&clone_url))?
        else {
            return Ok(ProjectRepoSyncStatusInfo {
                local_path: None,
                local_branch: None,
                local_head: None,
                local_short_head: None,
                remote_branch: default_branch
                    .as_deref()
                    .and_then(|branch| normalize_branch_option(Some(branch))),
                remote_head: None,
                remote_short_head: None,
                ahead_count: 0,
                behind_count: 0,
                has_uncommitted_changes: false,
                has_untracked_files: false,
                can_push: false,
                push_block_reason: Some("No local checkout found.".to_string()),
            });
        };

        Ok(compare_local_remote_status(
            &repo_dir,
            &clone_url,
            default_branch.as_deref(),
            &auth,
        ))
    })
    .await
    .map_err(|error| format!("repo sync status task failed: {error}"))?
}

#[tauri::command]
pub async fn push_project_local_repository(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: String,
    default_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoPushResult, String> {
    validate_clone_url(&clone_url)?;
    let auth = build_git_auth_config(&state)?;

    tauri::async_runtime::spawn_blocking(move || {
        let Some(repo_dir) =
            find_local_repo_dir(repos_dir.as_deref(), &project_dtag, Some(&clone_url))?
        else {
            return Err("No local checkout found.".to_string());
        };
        let status =
            compare_local_remote_status(&repo_dir, &clone_url, default_branch.as_deref(), &auth);
        if !status.can_push {
            return Err(status
                .push_block_reason
                .unwrap_or_else(|| "Local checkout cannot be pushed.".to_string()));
        }
        let branch = status
            .remote_branch
            .as_deref()
            .ok_or_else(|| "No branch selected for push.".to_string())?;
        run_git(
            &["push", "origin", format!("HEAD:{branch}").as_str()],
            Some(&repo_dir),
            &auth,
        )?;

        Ok(ProjectRepoPushResult {
            pushed: true,
            message: format!("Pushed {branch} to remote."),
        })
    })
    .await
    .map_err(|error| format!("repo push task failed: {error}"))?
}
