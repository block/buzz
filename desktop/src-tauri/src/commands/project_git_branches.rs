//! Authenticated remote branch management for Projects repositories.

use super::project_git::first_output_line;
use super::project_git_diff::clean_commit;
use super::project_git_exec::{
    build_git_auth_config, clean_branch, run_git, validate_workspace_clone_url, GitAuthConfig,
};
use crate::app_state::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ProjectRepoBranchResult {
    pub branch: String,
    pub commit: String,
    pub message: String,
}

fn normalize_branch(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.starts_with("refs/") && !value.starts_with("refs/heads/") {
        return Err(format!("Invalid {label} branch."));
    }
    let branch =
        clean_branch(Some(value.to_string())).ok_or_else(|| format!("Invalid {label} branch."))?;
    if branch.ends_with('.')
        || branch.ends_with(".lock")
        || branch.contains("//")
        || branch
            .split('/')
            .any(|component| component.starts_with('.'))
    {
        return Err(format!("Invalid {label} branch."));
    }
    Ok(branch)
}

fn normalize_commit(value: &str, label: &str) -> Result<String, String> {
    clean_commit(Some(value.trim().to_ascii_lowercase()))
        .ok_or_else(|| format!("Invalid {label} commit."))
}

fn remote_head_branch(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let target = line.strip_prefix("ref: ")?.strip_suffix("\tHEAD")?;
        let branch = target.strip_prefix("refs/heads/")?;
        clean_branch(Some(branch.to_string()))
    })
}

fn create_remote_branch_blocking(
    clone_url: &str,
    source_branch: &str,
    expected_commit: &str,
    new_branch: &str,
    auth: &GitAuthConfig,
) -> Result<ProjectRepoBranchResult, String> {
    let temp_dir = tempfile::tempdir().map_err(|error| format!("create temp dir: {error}"))?;
    let repo_dir = temp_dir.path().join("repo.git");
    let repo_path = repo_dir
        .to_str()
        .ok_or_else(|| "temporary repository path is not UTF-8".to_string())?;
    run_git(&["init", "--bare", "--", repo_path], None, auth)?;
    run_git(
        &["remote", "add", "--", "origin", clone_url],
        Some(&repo_dir),
        auth,
    )?;
    run_git(
        &[
            "fetch",
            "--quiet",
            "--depth=1",
            "--no-tags",
            "--end-of-options",
            "origin",
            format!("refs/heads/{source_branch}").as_str(),
        ],
        Some(&repo_dir),
        auth,
    )?;
    let source_commit = run_git(&["rev-parse", "FETCH_HEAD"], Some(&repo_dir), auth)
        .ok()
        .and_then(|output| first_output_line(&output))
        .ok_or_else(|| "Could not resolve the source branch.".to_string())?
        .to_ascii_lowercase();
    if source_commit != expected_commit {
        return Err(
            "The source branch changed. Refresh the repository before creating a branch."
                .to_string(),
        );
    }
    let lease = format!("--force-with-lease=refs/heads/{new_branch}:");
    let refspec = format!("{source_commit}:refs/heads/{new_branch}");
    run_git(
        &[
            "push",
            lease.as_str(),
            "--end-of-options",
            "origin",
            refspec.as_str(),
        ],
        Some(&repo_dir),
        auth,
    )?;

    Ok(ProjectRepoBranchResult {
        branch: new_branch.to_string(),
        commit: source_commit,
        message: format!("Created branch {new_branch} from {source_branch}."),
    })
}

fn delete_remote_branch_blocking(
    clone_url: &str,
    branch: &str,
    expected_commit: &str,
    auth: &GitAuthConfig,
) -> Result<ProjectRepoBranchResult, String> {
    let head_output = run_git(
        &[
            "ls-remote",
            "--symref",
            "--exit-code",
            "--end-of-options",
            clone_url,
            "HEAD",
        ],
        None,
        auth,
    )?;
    if remote_head_branch(&head_output).as_deref() == Some(branch) {
        return Err("The repository's default branch cannot be deleted.".to_string());
    }

    let temp_dir = tempfile::tempdir().map_err(|error| format!("create temp dir: {error}"))?;
    let repo_dir = temp_dir.path().join("repo.git");
    let repo_path = repo_dir
        .to_str()
        .ok_or_else(|| "temporary repository path is not UTF-8".to_string())?;
    run_git(&["init", "--bare", "--", repo_path], None, auth)?;
    run_git(
        &["remote", "add", "--", "origin", clone_url],
        Some(&repo_dir),
        auth,
    )?;
    let lease = format!("--force-with-lease=refs/heads/{branch}:{expected_commit}");
    let refspec = format!(":refs/heads/{branch}");
    run_git(
        &[
            "push",
            lease.as_str(),
            "--end-of-options",
            "origin",
            refspec.as_str(),
        ],
        Some(&repo_dir),
        auth,
    )?;

    Ok(ProjectRepoBranchResult {
        branch: branch.to_string(),
        commit: expected_commit.to_string(),
        message: format!("Deleted branch {branch}."),
    })
}

#[tauri::command]
pub async fn create_project_remote_branch(
    clone_url: String,
    source_branch: String,
    expected_commit: String,
    new_branch: String,
    state: State<'_, AppState>,
) -> Result<ProjectRepoBranchResult, String> {
    validate_workspace_clone_url(&clone_url, &state)?;
    let source_branch = normalize_branch(&source_branch, "source")?;
    let expected_commit = normalize_commit(&expected_commit, "source")?;
    let new_branch = normalize_branch(&new_branch, "new")?;
    if new_branch == source_branch {
        return Err("The new branch must have a different name.".to_string());
    }
    let auth = build_git_auth_config(&state)?;

    tauri::async_runtime::spawn_blocking(move || {
        create_remote_branch_blocking(
            &clone_url,
            &source_branch,
            &expected_commit,
            &new_branch,
            &auth,
        )
    })
    .await
    .map_err(|error| format!("branch creation task failed: {error}"))?
}

#[tauri::command]
pub async fn delete_project_remote_branch(
    clone_url: String,
    branch: String,
    expected_commit: String,
    state: State<'_, AppState>,
) -> Result<ProjectRepoBranchResult, String> {
    validate_workspace_clone_url(&clone_url, &state)?;
    let branch = normalize_branch(&branch, "branch")?;
    let expected_commit = normalize_commit(&expected_commit, "branch")?;
    let auth = build_git_auth_config(&state)?;

    tauri::async_runtime::spawn_blocking(move || {
        delete_remote_branch_blocking(&clone_url, &branch, &expected_commit, &auth)
    })
    .await
    .map_err(|error| format!("branch deletion task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{
        create_remote_branch_blocking, delete_remote_branch_blocking, normalize_branch,
        normalize_commit, remote_head_branch,
    };
    use crate::commands::project_git_exec::{build_test_git_auth_config, run_git};

    #[test]
    fn branch_inputs_use_conservative_git_validation() {
        assert_eq!(
            normalize_branch("refs/heads/feature/demo", "source"),
            Ok("feature/demo".to_string())
        );
        assert!(normalize_branch("--upload-pack=/tmp/evil", "new").is_err());
        assert!(normalize_branch("feature/../main", "new").is_err());
        assert!(normalize_branch("refs/tags/v1", "new").is_err());
        assert!(normalize_branch("feature//demo", "new").is_err());
        assert!(normalize_branch("feature/.hidden", "new").is_err());
        assert!(normalize_branch("feature/demo.lock", "new").is_err());
    }

    #[test]
    fn commit_inputs_accept_sha1_and_sha256() {
        assert_eq!(
            normalize_commit(&"A".repeat(40), "source"),
            Ok("a".repeat(40))
        );
        assert_eq!(
            normalize_commit(&"B".repeat(64), "branch"),
            Ok("b".repeat(64))
        );
        assert!(normalize_commit("not-a-commit", "source").is_err());
    }

    #[test]
    fn remote_head_parser_only_accepts_branch_symrefs() {
        assert_eq!(
            remote_head_branch(
                "ref: refs/heads/main\tHEAD\n0123456789012345678901234567890123456789\tHEAD\n"
            ),
            Some("main".to_string())
        );
        assert_eq!(remote_head_branch("0123\tHEAD\n"), None);
        assert_eq!(remote_head_branch("ref: refs/tags/v1\tHEAD\n"), None);
    }

    #[test]
    fn remote_branch_create_and_delete_round_trip() {
        let auth = build_test_git_auth_config().expect("build test git config");
        let root = tempfile::tempdir().expect("create test directory");
        let remote = root.path().join("remote.git");
        let worktree = root.path().join("worktree");
        let remote_path = remote.to_str().expect("remote path");
        let worktree_path = worktree.to_str().expect("worktree path");

        run_git(&["init", "--bare", "--", remote_path], None, &auth).expect("init remote");
        run_git(&["init", "--", worktree_path], None, &auth).expect("init worktree");
        std::fs::write(worktree.join("README.md"), "branch test\n").expect("write fixture");
        run_git(&["add", "README.md"], Some(&worktree), &auth).expect("stage fixture");
        run_git(
            &[
                "-c",
                "user.name=Buzz Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "Initial commit",
            ],
            Some(&worktree),
            &auth,
        )
        .expect("commit fixture");
        run_git(&["branch", "-M", "main"], Some(&worktree), &auth).expect("rename branch");
        run_git(
            &["remote", "add", "origin", remote_path],
            Some(&worktree),
            &auth,
        )
        .expect("add remote");
        run_git(&["push", "origin", "main"], Some(&worktree), &auth).expect("push main");
        run_git(
            &[
                format!("--git-dir={remote_path}").as_str(),
                "symbolic-ref",
                "HEAD",
                "refs/heads/main",
            ],
            None,
            &auth,
        )
        .expect("set remote HEAD");
        let commit = run_git(&["rev-parse", "HEAD"], Some(&worktree), &auth)
            .expect("resolve fixture commit")
            .trim()
            .to_string();

        let created =
            create_remote_branch_blocking(remote_path, "main", &commit, "feature/demo", &auth)
                .expect("create remote branch");
        assert_eq!(created.branch, "feature/demo");
        assert!(run_git(
            &[
                format!("--git-dir={remote_path}").as_str(),
                "show-ref",
                "--verify",
                "refs/heads/feature/demo",
            ],
            None,
            &auth,
        )
        .is_ok());

        delete_remote_branch_blocking(remote_path, "feature/demo", &commit, &auth)
            .expect("delete remote branch");
        assert!(run_git(
            &[
                format!("--git-dir={remote_path}").as_str(),
                "show-ref",
                "--verify",
                "refs/heads/feature/demo",
            ],
            None,
            &auth,
        )
        .is_err());
        assert!(delete_remote_branch_blocking(remote_path, "main", &commit, &auth).is_err());
    }
}
