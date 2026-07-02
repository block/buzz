//! Opens an OS terminal window at a project's local git checkout, cloning
//! the repository from the relay first when no local checkout exists.

use serde::Serialize;
use std::process::Command;
use tauri::State;

use crate::app_state::AppState;

use super::project_git::{
    build_git_auth_config, normalize_branch_option, run_git, validate_clone_url,
};
use super::project_repo_paths::{
    canonical_repos_roots, canonicalize_repos_root, default_repos_root_candidates,
    find_local_repo_dir, local_repo_candidates,
};

/// Result of [`open_project_terminal`]: where the terminal opened and
/// whether a fresh clone was made to get there.
#[derive(Serialize)]
pub struct ProjectTerminalResult {
    pub path: String,
    pub cloned: bool,
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

/// Resolves the repos root a fresh clone should land in, creating the
/// default root when no explicit `reposDir` is configured and none of the
/// default candidates exist yet.
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
    // Auth is only needed for the clone path — a missing git binary must not
    // block opening a terminal at an existing checkout.
    let auth = build_git_auth_config(&state);
    let branch = normalize_branch_option(default_branch.as_deref());

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
        validate_clone_url(clone_url)?;
        let auth = auth?;

        let repos_root = clone_destination_root(repos_dir.as_deref())?;
        let repo_name = local_repo_candidates(&project_dtag, Some(clone_url))
            .into_iter()
            .next()
            .ok_or_else(|| "Could not derive a directory name for the repository.".to_string())?;
        let repo_dir = repos_root.join(&repo_name);
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
            clone_args.push("--branch");
            clone_args.push(branch.as_str());
        }
        clone_args.push(clone_url);
        clone_args.push(repo_path);
        if let Err(error) = run_git(&clone_args, None, &auth) {
            // The requested branch may not exist on the remote — retry with
            // the remote's default branch before giving up.
            if branch.is_none() {
                return Err(error);
            }
            run_git(&["clone", clone_url, repo_path], None, &auth)?;
        }

        launch_terminal_at(&repo_dir)?;
        Ok(ProjectTerminalResult {
            path: repo_dir.display().to_string(),
            cloned: true,
        })
    })
    .await
    .map_err(|error| format!("open terminal task failed: {error}"))?
}
