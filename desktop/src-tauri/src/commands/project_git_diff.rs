use crate::{
    app_state::AppState,
    managed_agents::{nest_dir, resolve_command},
};
use nostr::ToBech32;
use serde::Serialize;
use std::process::Command;
use tauri::State;
use url::Url;

#[derive(Serialize)]
pub struct ProjectRepoDiffFileInfo {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
}

#[derive(Serialize)]
pub struct ProjectRepoDiffInfo {
    pub files: Vec<ProjectRepoDiffFileInfo>,
    pub additions: usize,
    pub deletions: usize,
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

fn clean_branch(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_start_matches("refs/heads/"))
        .filter(|value| {
            !value.contains("..")
                && !value.starts_with('/')
                && !value.ends_with('/')
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
        })
        .map(ToString::to_string)
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

fn clean_commit(value: Option<String>) -> Option<String> {
    value
        .filter(|value| matches!(value.len(), 40 | 64))
        .filter(|value| value.chars().all(|c| c.is_ascii_hexdigit()))
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

fn default_repos_root_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(nest_dir().map(|path| path.join("REPOS")));
    candidates.extend(
        dirs::home_dir()
            .map(|home| home.join(".buzz").join("REPOS"))
            .filter(|path| !candidates.iter().any(|candidate| candidate == path)),
    );
    candidates
}

fn canonicalize_repos_root(repos_root: std::path::PathBuf) -> Result<std::path::PathBuf, String> {
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

fn canonical_repos_roots(repos_dir: Option<&str>) -> Result<Vec<std::path::PathBuf>, String> {
    if let Some(repos_root) = repos_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
    {
        return canonicalize_repos_root(repos_root).map(|root| vec![root]);
    }
    let roots = default_repos_root_candidates()
        .into_iter()
        .filter_map(|root| canonicalize_repos_root(root).ok())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err("reposDir is not accessible".to_string());
    }
    Ok(roots)
}

fn find_local_repo_dir(
    repos_dir: Option<&str>,
    project_dtag: &str,
    clone_url: Option<&str>,
) -> Result<Option<std::path::PathBuf>, String> {
    for repos_root in canonical_repos_roots(repos_dir)? {
        for candidate in local_repo_candidates(project_dtag, clone_url) {
            let candidate_path = repos_root.join(candidate);
            let Ok(candidate_path) = candidate_path.canonicalize() else {
                continue;
            };
            if candidate_path.starts_with(&repos_root)
                && candidate_path.is_dir()
                && candidate_path.join(".git").exists()
            {
                return Ok(Some(candidate_path));
            }
        }
    }
    Ok(None)
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
    empty_tree_ref(repo_dir, auth)
        .map(|empty_tree| format!("{empty_tree}..{target_ref}"))
        .unwrap_or_else(|_| format!("{target_ref}^..{target_ref}"))
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
            ProjectRepoDiffFileInfo {
                path,
                additions,
                deletions,
                patch,
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
    validate_clone_url(&clone_url)?;
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
        let range = diff_range(
            &repo_dir,
            &auth,
            diff_base_ref(&repo_dir, &auth, base_branch.as_deref()),
        );
        diff_from_repo(&repo_dir, &auth, &range)
    })
    .await
    .map_err(|error| format!("repo diff task failed: {error}"))?
}

#[tauri::command]
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
