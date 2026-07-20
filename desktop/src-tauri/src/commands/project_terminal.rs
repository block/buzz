//! Opens an OS terminal window at a project's local git checkout, cloning
//! the repository from the relay first when no local checkout exists.

use serde::{Deserialize, Serialize};
use std::process::Command;
use tauri::State;

use crate::app_state::AppState;

use super::project_git::{first_output_line, normalize_branch_option};
use super::project_git_diff::clean_commit;
use super::project_git_exec::{build_git_auth_config, run_git, validate_workspace_clone_url};
use super::project_git_workflow::clone_project_repository_blocking;
use super::project_repo_paths::find_local_repo_dir;

/// Result of [`open_project_terminal`]: where the terminal opened and
/// whether a fresh clone was made to get there.
#[derive(Serialize)]
pub struct ProjectTerminalResult {
    pub path: String,
    pub cloned: bool,
}

/// Inputs for preparing an authenticated local merge-conflict recovery.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMergeRecoveryTerminalInput {
    repos_dir: Option<String>,
    project_dtag: String,
    target_clone_url: String,
    source_clone_url: String,
    target_branch: String,
    source_branch: String,
    expected_commit: String,
}

/// Local checkout and ref prepared for terminal-based conflict resolution.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMergeRecoveryTerminalResult {
    path: String,
    cloned: bool,
    recovery_ref: String,
    target_ref: String,
}

fn merge_recovery_ref(expected_commit: &str) -> String {
    format!("refs/buzz/merge-recovery/{expected_commit}")
}

fn merge_recovery_target_ref(target_commit: &str) -> String {
    format!("refs/buzz/merge-recovery-target/{target_commit}")
}

#[cfg(target_os = "macos")]
fn launch_terminal_at(path: &std::path::Path) -> Result<(), String> {
    let status = Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(path)
        .status()
        .map_err(|error| format!("failed to open Terminal: {error}"))?;
    if !status.success() {
        return Err("failed to open Terminal".to_string());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch_terminal_at(path: &std::path::Path) -> Result<(), String> {
    // Try common terminal emulators in order; each inherits the repo dir as cwd.
    let candidates: [(&str, &[&str]); 4] = [
        ("x-terminal-emulator", &[]),
        ("gnome-terminal", &[]),
        ("konsole", &[]),
        ("xterm", &[]),
    ];
    for (command, args) in candidates {
        if Command::new(command)
            .args(args)
            .current_dir(path)
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("no terminal emulator found".to_string())
}

#[cfg(target_os = "windows")]
fn launch_terminal_at(path: &std::path::Path) -> Result<(), String> {
    Command::new("cmd")
        .args(["/C", "start", "", "cmd"])
        .current_dir(path)
        .spawn()
        .map_err(|error| format!("failed to open terminal: {error}"))?;
    Ok(())
}

/// Opens the OS terminal at the project's local checkout. When there is no
/// local checkout yet, clones the repository from `clone_url` (authenticated
/// with the identity key, same as push/snapshot) into the repos dir first,
/// then opens the terminal at the fresh checkout.
#[tauri::command]
pub async fn open_project_terminal(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: Option<String>,
    default_branch: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectTerminalResult, String> {
    if let Some(clone_url) = clone_url.as_deref() {
        validate_workspace_clone_url(clone_url, &state)?;
    }
    // Auth is only needed for the clone path — keep the result outside the
    // blocking task so it owns no borrowed Tauri state.
    let auth = build_git_auth_config(&state);
    tauri::async_runtime::spawn_blocking(move || {
        // An inaccessible repos root (fresh machine, nothing cloned yet) is
        // not fatal here — the clone path below creates the default root. A
        // misconfigured explicit reposDir still errors in clone_destination_root.
        let local_dir =
            find_local_repo_dir(repos_dir.as_deref(), &project_dtag, clone_url.as_deref())
                .ok()
                .flatten();
        if let Some(repo_dir) = local_dir {
            launch_terminal_at(&repo_dir)?;
            return Ok(ProjectTerminalResult {
                path: repo_dir.display().to_string(),
                cloned: false,
            });
        }

        let clone_url = clone_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "No local checkout and no clone URL available.".to_string())?;
        let auth = auth?;
        let clone_result = clone_project_repository_blocking(
            repos_dir.as_deref(),
            &project_dtag,
            clone_url,
            default_branch.as_deref(),
            &auth,
        )?;
        let repo_dir = std::path::PathBuf::from(&clone_result.path);
        launch_terminal_at(&repo_dir)?;
        Ok(ProjectTerminalResult {
            path: clone_result.path,
            cloned: clone_result.cloned,
        })
    })
    .await
    .map_err(|error| format!("open terminal task failed: {error}"))?
}

/// Authenticates and fetches the exact pull-request commit before opening a
/// terminal. The user's worktree is not switched or modified.
#[tauri::command]
pub async fn open_project_merge_recovery_terminal(
    input: ProjectMergeRecoveryTerminalInput,
    state: State<'_, AppState>,
) -> Result<ProjectMergeRecoveryTerminalResult, String> {
    validate_workspace_clone_url(&input.target_clone_url, &state)?;
    validate_workspace_clone_url(&input.source_clone_url, &state)?;
    let target_branch = normalize_branch_option(Some(&input.target_branch))
        .ok_or_else(|| "Invalid target branch.".to_string())?;
    let source_branch = normalize_branch_option(Some(&input.source_branch))
        .ok_or_else(|| "Invalid source branch.".to_string())?;
    let expected_commit = clean_commit(Some(input.expected_commit.trim().to_ascii_lowercase()))
        .ok_or_else(|| "Invalid pull request commit.".to_string())?;
    let auth = build_git_auth_config(&state)?;

    tauri::async_runtime::spawn_blocking(move || {
        let existing_dir = find_local_repo_dir(
            input.repos_dir.as_deref(),
            &input.project_dtag,
            Some(&input.target_clone_url),
        )
        .ok()
        .flatten();
        let (repo_dir, cloned) = if let Some(repo_dir) = existing_dir {
            (repo_dir, false)
        } else {
            let clone_result = clone_project_repository_blocking(
                input.repos_dir.as_deref(),
                &input.project_dtag,
                &input.target_clone_url,
                Some(&target_branch),
                &auth,
            )?;
            (std::path::PathBuf::from(clone_result.path), true)
        };

        run_git(
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--end-of-options",
                input.target_clone_url.as_str(),
                target_branch.as_str(),
            ],
            Some(&repo_dir),
            &auth,
        )?;
        let target_head = run_git(&["rev-parse", "FETCH_HEAD"], Some(&repo_dir), &auth)
            .ok()
            .and_then(|output| first_output_line(&output))
            .ok_or_else(|| "Could not resolve the target branch.".to_string())?;
        let target_ref = merge_recovery_target_ref(&target_head);
        run_git(
            &["update-ref", target_ref.as_str(), target_head.as_str()],
            Some(&repo_dir),
            &auth,
        )?;

        run_git(
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--end-of-options",
                input.source_clone_url.as_str(),
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
                "The pull request branch changed. Refresh the pull request before resolving conflicts."
                    .to_string(),
            );
        }

        let recovery_ref = merge_recovery_ref(&expected_commit);
        run_git(
            &["update-ref", recovery_ref.as_str(), expected_commit.as_str()],
            Some(&repo_dir),
            &auth,
        )?;
        launch_terminal_at(&repo_dir)?;
        Ok(ProjectMergeRecoveryTerminalResult {
            path: repo_dir.display().to_string(),
            cloned,
            recovery_ref,
            target_ref,
        })
    })
    .await
    .map_err(|error| format!("open merge recovery terminal task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{merge_recovery_ref, merge_recovery_target_ref};

    #[test]
    fn recovery_ref_is_namespaced_by_verified_commit() {
        let commit = "a".repeat(40);
        assert_eq!(
            merge_recovery_ref(&commit),
            format!("refs/buzz/merge-recovery/{commit}"),
        );
        assert_eq!(
            merge_recovery_target_ref(&commit),
            format!("refs/buzz/merge-recovery-target/{commit}"),
        );
    }
}
