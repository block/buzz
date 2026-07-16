//! Local clone and pull-request merge commands for the Projects workflow.

use super::project_git::{first_output_line, normalize_branch_option};
use super::project_git_diff::clean_commit;
use super::project_git_exec::{
    build_git_auth_config, clone_url_owner, run_git, validate_clone_url,
    validate_workspace_clone_url, GitAuthConfig,
};
use super::project_repo_paths::{
    canonical_repos_roots, canonicalize_repos_root, default_repos_root_candidates,
    find_local_repo_dir, local_repo_candidates,
};
use crate::app_state::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ProjectRepoCloneResult {
    pub path: String,
    pub cloned: bool,
    pub message: String,
}

#[derive(Serialize)]
pub struct ProjectRepoMergeResult {
    pub message: String,
    pub merge_commit: String,
}

fn normalize_commit(value: &str) -> Option<String> {
    clean_commit(Some(value.trim().to_ascii_lowercase()))
}

fn same_repository(left: &str, right: &str) -> bool {
    left.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .eq_ignore_ascii_case(right.trim().trim_end_matches('/').trim_end_matches(".git"))
}

fn clone_destination_root(repos_dir: Option<&str>) -> Result<std::path::PathBuf, String> {
    match canonical_repos_roots(repos_dir) {
        Ok(roots) => roots
            .into_iter()
            .next()
            .ok_or_else(|| "reposDir is not accessible".to_string()),
        Err(error) => {
            if repos_dir.is_some() {
                return Err(error);
            }
            let root = default_repos_root_candidates()
                .into_iter()
                .next()
                .ok_or(error)?;
            std::fs::create_dir_all(&root).map_err(|error| format!("create repos dir: {error}"))?;
            canonicalize_repos_root(root)
        }
    }
}

pub(crate) fn clone_project_repository_blocking(
    repos_dir: Option<&str>,
    project_dtag: &str,
    clone_url: &str,
    default_branch: Option<&str>,
    auth: &GitAuthConfig,
) -> Result<ProjectRepoCloneResult, String> {
    validate_clone_url(clone_url)?;
    let branch = normalize_branch_option(default_branch);
    if let Some(repo_dir) = find_local_repo_dir(repos_dir, project_dtag, Some(clone_url))? {
        return Ok(ProjectRepoCloneResult {
            path: repo_dir.display().to_string(),
            cloned: false,
            message: "Repository is already cloned.".to_string(),
        });
    }

    let repos_root = clone_destination_root(repos_dir)?;
    let repo_name = local_repo_candidates(project_dtag, Some(clone_url))
        .into_iter()
        .next()
        .ok_or_else(|| "Could not derive a directory name for the repository.".to_string())?;
    let repo_dir = repos_root.join(repo_name);
    if repo_dir.exists() {
        return Err(format!(
            "{} already exists but is not a git checkout.",
            repo_dir.display()
        ));
    }
    let repo_path = repo_dir
        .to_str()
        .ok_or_else(|| "repository path is not UTF-8".to_string())?;

    let mut clone_args = vec!["clone"];
    if let Some(ref branch) = branch {
        clone_args.extend(["--branch", branch.as_str()]);
    }
    clone_args.extend(["--end-of-options", clone_url, repo_path]);
    if let Err(error) = run_git(&clone_args, None, auth) {
        if branch.is_none() {
            return Err(error);
        }
        run_git(
            &["clone", "--end-of-options", clone_url, repo_path],
            None,
            auth,
        )?;
    }

    Ok(ProjectRepoCloneResult {
        path: repo_dir.display().to_string(),
        cloned: true,
        message: format!("Cloned repository to {}.", repo_dir.display()),
    })
}

#[tauri::command]
pub async fn clone_project_repository(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: String,
    default_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoCloneResult, String> {
    validate_workspace_clone_url(&clone_url, &state)?;
    let auth = build_git_auth_config(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        clone_project_repository_blocking(
            repos_dir.as_deref(),
            &project_dtag,
            &clone_url,
            default_branch.as_deref(),
            &auth,
        )
    })
    .await
    .map_err(|error| format!("repo clone task failed: {error}"))?
}

#[tauri::command]
pub async fn merge_project_pull_request(
    target_clone_url: String,
    source_clone_url: String,
    target_owner: String,
    target_branch: String,
    source_branch: String,
    expected_commit: String,
    state: State<'_, AppState>,
) -> Result<ProjectRepoMergeResult, String> {
    validate_workspace_clone_url(&target_clone_url, &state)?;
    validate_workspace_clone_url(&source_clone_url, &state)?;
    let target_owner = target_owner.trim().to_ascii_lowercase();
    if target_owner.len() != 64 || !target_owner.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid target repository owner.".to_string());
    }
    if clone_url_owner(&target_clone_url).as_deref() != Some(target_owner.as_str()) {
        return Err("Target clone URL does not match the repository owner.".to_string());
    }
    let merger_pubkey = state
        .keys
        .lock()
        .map_err(|error| error.to_string())?
        .public_key()
        .to_hex();
    if merger_pubkey != target_owner {
        return Err("Only the repository owner can merge pull requests.".to_string());
    }
    let target_branch = normalize_branch_option(Some(&target_branch))
        .ok_or_else(|| "Invalid target branch.".to_string())?;
    let source_branch = normalize_branch_option(Some(&source_branch))
        .ok_or_else(|| "Invalid source branch.".to_string())?;
    if target_branch == source_branch && same_repository(&target_clone_url, &source_clone_url) {
        return Err("Source and target branches must be different.".to_string());
    }
    let expected_commit = normalize_commit(&expected_commit)
        .ok_or_else(|| "Invalid pull request commit.".to_string())?;
    let auth = build_git_auth_config(&state)?;

    tauri::async_runtime::spawn_blocking(move || {
        let temp_dir = tempfile::tempdir().map_err(|error| format!("create temp dir: {error}"))?;
        let repo_dir = temp_dir.path().join("repo");
        let repo_path = repo_dir
            .to_str()
            .ok_or_else(|| "temporary repository path is not UTF-8".to_string())?;
        run_git(
            &[
                "clone",
                "--filter=blob:none",
                "--no-tags",
                "--branch",
                target_branch.as_str(),
                "--single-branch",
                "--end-of-options",
                target_clone_url.as_str(),
                repo_path,
            ],
            None,
            &auth,
        )?;
        run_git(
            &[
                "fetch",
                "--quiet",
                "--depth=100",
                "--end-of-options",
                source_clone_url.as_str(),
                source_branch.as_str(),
            ],
            Some(&repo_dir),
            &auth,
        )?;
        let source_head = run_git(&["rev-parse", "FETCH_HEAD"], Some(&repo_dir), &auth)
            .ok()
            .and_then(|output| first_output_line(&output))
            .ok_or_else(|| "Could not resolve the pull request branch.".to_string())?;
        if source_head.to_ascii_lowercase() != expected_commit {
            return Err(
                "The pull request branch changed. Refresh the pull request before merging."
                    .to_string(),
            );
        }

        let merge_email = format!("{merger_pubkey}@users.noreply.buzz");
        run_git(
            &[
                "-c",
                "user.name=Buzz User",
                "-c",
                format!("user.email={merge_email}").as_str(),
                "merge",
                "--no-edit",
                "--end-of-options",
                expected_commit.as_str(),
            ],
            Some(&repo_dir),
            &auth,
        )
        .map_err(|error| format!("Pull request cannot be merged cleanly: {error}"))?;
        let merge_commit = run_git(&["rev-parse", "HEAD"], Some(&repo_dir), &auth)
            .ok()
            .and_then(|output| first_output_line(&output))
            .ok_or_else(|| "Could not resolve the merge commit.".to_string())?;
        run_git(
            &[
                "push",
                "--end-of-options",
                "origin",
                format!("HEAD:{target_branch}").as_str(),
            ],
            Some(&repo_dir),
            &auth,
        )?;

        Ok(ProjectRepoMergeResult {
            message: format!("Merged {source_branch} into {target_branch}."),
            merge_commit,
        })
    })
    .await
    .map_err(|error| format!("pull request merge task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{normalize_commit, same_repository};

    #[test]
    fn normalize_commit_accepts_sha1_and_sha256_hex() {
        assert_eq!(normalize_commit(&"A".repeat(40)), Some("a".repeat(40)));
        assert_eq!(normalize_commit(&"B".repeat(64)), Some("b".repeat(64)));
    }

    #[test]
    fn normalize_commit_rejects_invalid_values() {
        assert_eq!(normalize_commit("abc"), None);
        assert_eq!(normalize_commit(&"z".repeat(40)), None);
    }

    #[test]
    fn repository_comparison_normalizes_git_suffix_and_trailing_slash() {
        assert!(same_repository(
            "https://relay.example/git/owner/repo.git",
            "https://relay.example/git/owner/repo/"
        ));
        assert!(!same_repository(
            "https://relay.example/git/owner/repo",
            "https://relay.example/git/fork/repo"
        ));
    }
}
