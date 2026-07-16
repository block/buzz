//! Opens an OS terminal window at a project's local git checkout, cloning
//! the repository from the relay first when no local checkout exists.

use serde::Serialize;
use std::process::Command;
use tauri::State;

use crate::app_state::AppState;

use super::project_git_exec::{build_git_auth_config, validate_workspace_clone_url};
use super::project_git_workflow::clone_project_repository_blocking;
use super::project_repo_paths::find_local_repo_dir;

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
