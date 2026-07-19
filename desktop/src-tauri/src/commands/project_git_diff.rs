use super::project_git_exec::{
    build_git_auth_config, clean_branch, run_git, validate_workspace_clone_url, GitAuthConfig,
};
use super::project_repo_paths::find_local_repo_dir;
use crate::app_state::AppState;
use serde::Serialize;
use tauri::State;

/// Per-file cap on rendered patch lines. One regenerated lockfile or
/// minified bundle would otherwise produce tens of thousands of DOM nodes
/// in the diff view and freeze the webview.
const MAX_PATCH_LINES: usize = 2_000;

#[derive(Serialize)]
pub struct ProjectRepoDiffFileInfo {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct ProjectRepoDiffInfo {
    pub files: Vec<ProjectRepoDiffFileInfo>,
    pub additions: usize,
    pub deletions: usize,
}

fn clean_target_ref(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        value.starts_with("refs/")
            && !value.contains("..")
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
    })
}

pub(crate) fn clean_commit(value: Option<String>) -> Option<String> {
    value
        .filter(|value| matches!(value.len(), 40 | 64))
        .filter(|value| value.chars().all(|c| c.is_ascii_hexdigit()))
}

fn fetch_target(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_ref: Option<&str>,
    target_commit: Option<&str>,
) -> Result<(), String> {
    if let Some(target_ref) = target_ref {
        if run_git(
            &["fetch", "--depth=100", "origin", target_ref],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    } else if let Some(target_commit) = target_commit {
        if run_git(
            &["fetch", "--depth=100", "origin", target_commit],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    }

    if let Some(target_commit) = target_commit {
        if run_git(
            &["fetch", "--depth=100", "origin", target_commit],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    }

    if let Some(branch) = branch {
        let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        run_git(
            &["fetch", "--depth=100", "origin", &refspec],
            Some(repo_dir),
            auth,
        )?;
        run_git(
            &["checkout", "--detach", &format!("origin/{branch}")],
            Some(repo_dir),
            auth,
        )?;
        return Ok(());
    }

    run_git(
        &["fetch", "--depth=100", "origin", "HEAD"],
        Some(repo_dir),
        auth,
    )?;
    run_git(
        &["checkout", "--detach", "FETCH_HEAD"],
        Some(repo_dir),
        auth,
    )?;
    Ok(())
}

fn diff_base_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_branch: Option<&str>,
) -> Option<String> {
    let base_branch = base_branch?;
    let refspec = format!("refs/heads/{base_branch}:refs/remotes/origin/{base_branch}");
    run_git(
        &["fetch", "--depth=100", "origin", &refspec],
        Some(repo_dir),
        auth,
    )
    .ok()?;
    Some(format!("origin/{base_branch}"))
}

fn parse_count(value: &str) -> usize {
    value.parse::<usize>().unwrap_or_default()
}

fn parse_numstat(output: &str) -> Vec<(String, usize, usize)> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let additions = parse_count(parts.next()?);
            let deletions = parse_count(parts.next()?);
            let path = parts.next()?.to_string();
            Some((path, additions, deletions))
        })
        .take(250)
        .collect()
}

fn empty_tree_ref(repo_dir: &std::path::Path, auth: &GitAuthConfig) -> Result<String, String> {
    run_git(
        &["hash-object", "-t", "tree", "/dev/null"],
        Some(repo_dir),
        auth,
    )
    .map(|output| output.trim().to_string())
}

fn diff_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_ref: Option<String>,
) -> String {
    if let Some(base_ref) = base_ref {
        return if run_git(&["merge-base", &base_ref, "HEAD"], Some(repo_dir), auth).is_ok() {
            format!("{base_ref}...HEAD")
        } else {
            format!("{base_ref}..HEAD")
        };
    }

    empty_tree_ref(repo_dir, auth)
        .map(|empty_tree| format!("{empty_tree}..HEAD"))
        .unwrap_or_else(|_| "HEAD^..HEAD".to_string())
}

/// Range for a single commit against its parent, used by the commit detail
/// view. Root commits fall back to the empty tree so the whole initial tree
/// renders as additions. Errors when the commit is not reachable in the
/// available history — diffing an unrelated ref instead would be misleading.
fn commit_parent_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    commit: &str,
) -> Result<String, String> {
    run_git(
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{commit}^{{commit}}"),
        ],
        Some(repo_dir),
        auth,
    )
    .map_err(|_| format!("commit {commit} was not found in the repository history"))?;
    let parent = format!("{commit}^");
    if run_git(
        &["rev-parse", "--verify", "--quiet", &parent],
        Some(repo_dir),
        auth,
    )
    .is_ok()
    {
        return Ok(format!("{parent}..{commit}"));
    }
    let empty_tree = empty_tree_ref(repo_dir, auth)?;
    Ok(format!("{empty_tree}..{commit}"))
}

fn local_ref_exists(repo_dir: &std::path::Path, auth: &GitAuthConfig, ref_name: &str) -> bool {
    run_git(
        &["rev-parse", "--verify", "--quiet", ref_name],
        Some(repo_dir),
        auth,
    )
    .is_ok()
}

fn local_target_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_commit: Option<&str>,
) -> String {
    if let Some(target_commit) = target_commit {
        if local_ref_exists(repo_dir, auth, target_commit) {
            return target_commit.to_string();
        }
    }
    if let Some(branch) = branch {
        if local_ref_exists(repo_dir, auth, branch) {
            return branch.to_string();
        }
        let origin_branch = format!("origin/{branch}");
        if local_ref_exists(repo_dir, auth, &origin_branch) {
            return origin_branch;
        }
    }
    "HEAD".to_string()
}

fn local_base_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_branch: Option<&str>,
) -> Option<String> {
    let branch = branch?;
    let origin_branch = format!("origin/{branch}");
    if local_ref_exists(repo_dir, auth, &origin_branch) {
        return Some(origin_branch);
    }
    if target_branch == Some(branch) {
        return None;
    }
    local_ref_exists(repo_dir, auth, branch).then_some(branch.to_string())
}

fn local_diff_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_branch: Option<&str>,
    target_branch: Option<&str>,
    base_commit: Option<&str>,
    target_commit: Option<&str>,
) -> String {
    let target_ref = local_target_ref(repo_dir, auth, target_branch, target_commit);
    if let Some(base_commit) = base_commit {
        if base_commit != target_ref && local_ref_exists(repo_dir, auth, base_commit) {
            return if run_git(
                &["merge-base", base_commit, &target_ref],
                Some(repo_dir),
                auth,
            )
            .is_ok()
            {
                format!("{base_commit}...{target_ref}")
            } else {
                format!("{base_commit}..{target_ref}")
            };
        }
    }
    if let Some(base_ref) = local_base_ref(repo_dir, auth, base_branch, target_branch) {
        return if run_git(
            &["merge-base", &base_ref, &target_ref],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            format!("{base_ref}...{target_ref}")
        } else {
            format!("{base_ref}..{target_ref}")
        };
    }
    // With no base at all, a bare commit means "diff against its parent"
    // (commit detail view) rather than against the whole tree.
    if base_commit.is_none() && base_branch.is_none() {
        if let Some(target_commit) = target_commit {
            if local_ref_exists(repo_dir, auth, target_commit) {
                if let Ok(range) = commit_parent_range(repo_dir, auth, target_commit) {
                    return range;
                }
            }
        }
    }
    empty_tree_ref(repo_dir, auth)
        .map(|empty_tree| format!("{empty_tree}..{target_ref}"))
        .unwrap_or_else(|_| format!("{target_ref}^..{target_ref}"))
}

/// Caps a patch at [`MAX_PATCH_LINES`], reporting whether it was cut.
fn truncate_patch(patch: String) -> (String, bool) {
    let mut line_starts = patch
        .char_indices()
        .filter(|(_, c)| *c == '\n')
        .map(|(index, _)| index);
    match line_starts.nth(MAX_PATCH_LINES - 1) {
        Some(cut_at) => (patch[..cut_at].to_string(), true),
        None => (patch, false),
    }
}

fn diff_from_repo(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    range: &str,
) -> Result<ProjectRepoDiffInfo, String> {
    let numstat = run_git(&["diff", "--numstat", range], Some(repo_dir), auth)?;
    let files = parse_numstat(&numstat)
        .into_iter()
        .map(|(path, additions, deletions)| {
            let patch = run_git(
                &[
                    "diff",
                    "--no-ext-diff",
                    "--find-renames",
                    "--find-copies",
                    "--unified=80",
                    "--src-prefix=a/",
                    "--dst-prefix=b/",
                    range,
                    "--",
                    &path,
                ],
                Some(repo_dir),
                auth,
            )
            .unwrap_or_default();
            let (patch, truncated) = truncate_patch(patch);
            ProjectRepoDiffFileInfo {
                path,
                additions,
                deletions,
                patch,
                truncated,
            }
        })
        .collect::<Vec<_>>();
    Ok(ProjectRepoDiffInfo {
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
        files,
    })
}

#[tauri::command]
pub async fn get_project_repo_diff(
    clone_url: String,
    default_branch: Option<String>,
    base_branch: Option<String>,
    target_ref: Option<String>,
    target_commit: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoDiffInfo, String> {
    validate_workspace_clone_url(&clone_url, &state)?;
    let auth = build_git_auth_config(&state)?;
    let branch = clean_branch(default_branch);
    let base_branch = clean_branch(base_branch);
    let target_ref = clean_target_ref(target_ref);
    let target_commit = clean_commit(target_commit);

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
                "--no-checkout",
                &clone_url,
                repo_path,
            ],
            None,
            &auth,
        )?;
        fetch_target(
            &repo_dir,
            &auth,
            branch.as_deref(),
            target_ref.as_deref(),
            target_commit.as_deref(),
        )?;
        // A commit with no base branch or target ref means "diff this commit
        // against its parent" (commit detail view), not "diff HEAD against a
        // base".
        let range = match (&target_ref, &base_branch, &target_commit) {
            (None, None, Some(commit)) => commit_parent_range(&repo_dir, &auth, commit)?,
            _ => diff_range(
                &repo_dir,
                &auth,
                diff_base_ref(&repo_dir, &auth, base_branch.as_deref()),
            ),
        };
        diff_from_repo(&repo_dir, &auth, &range)
    })
    .await
    .map_err(|error| format!("repo diff task failed: {error}"))?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_project_local_repo_diff(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: Option<String>,
    default_branch: Option<String>,
    base_branch: Option<String>,
    base_commit: Option<String>,
    target_commit: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<ProjectRepoDiffInfo>, String> {
    let auth = build_git_auth_config(&state)?;
    let branch = clean_branch(default_branch);
    let base_branch = clean_branch(base_branch);
    let base_commit = clean_commit(base_commit);
    let target_commit = clean_commit(target_commit);

    tauri::async_runtime::spawn_blocking(move || {
        let Some(repo_dir) =
            find_local_repo_dir(repos_dir.as_deref(), &project_dtag, clone_url.as_deref())?
        else {
            return Ok(None);
        };
        let range = local_diff_range(
            &repo_dir,
            &auth,
            base_branch.as_deref(),
            branch.as_deref(),
            base_commit.as_deref(),
            target_commit.as_deref(),
        );
        diff_from_repo(&repo_dir, &auth, &range).map(Some)
    })
    .await
    .map_err(|error| format!("local repo diff task failed: {error}"))?
}
