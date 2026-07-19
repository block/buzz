//! Resolution of local project checkouts under the configured repos roots.
//!
//! Shared by the project git commands (snapshots, sync status, push) and the
//! project terminal launcher.

use crate::managed_agents::nest_dir;
use url::Url;

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
    let last_segment = parsed.path_segments()?.rfind(|part| !part.is_empty())?;
    local_repo_name_candidate(last_segment)
}

fn clone_url_owner_repo_name(clone_url: &str) -> Option<String> {
    let parsed = Url::parse(clone_url).ok()?;
    let parts = parsed
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let [.., owner, repo] = parts.as_slice() else {
        return None;
    };
    local_repo_name_candidate(&format!(
        "{}--{}",
        local_repo_name_candidate(owner)?,
        local_repo_name_candidate(repo)?
    ))
}

fn normalized_clone_url(value: &str) -> &str {
    value.trim().trim_end_matches('/').trim_end_matches(".git")
}

fn checkout_git_config(
    repo_dir: &std::path::Path,
    repos_root: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let dot_git = repo_dir.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let pointer = std::fs::read_to_string(dot_git).ok()?;
        let git_dir = std::path::PathBuf::from(pointer.trim().strip_prefix("gitdir:")?.trim());
        if git_dir.is_absolute() {
            git_dir
        } else {
            repo_dir.join(git_dir)
        }
    };
    let git_dir = git_dir.canonicalize().ok()?;
    if !git_dir.starts_with(repos_root) {
        return None;
    }
    let config = git_dir.join("config").canonicalize().ok()?;
    config.starts_with(repos_root).then_some(config)
}

fn checkout_origin_matches(
    repo_dir: &std::path::Path,
    repos_root: &std::path::Path,
    clone_url: &str,
) -> bool {
    let Some(config_path) = checkout_git_config(repo_dir, repos_root) else {
        return false;
    };
    let Ok(config) = std::fs::read_to_string(config_path) else {
        return false;
    };
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line == r#"[remote "origin"]"#;
            continue;
        }
        if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "url" {
                    return normalized_clone_url(value) == normalized_clone_url(clone_url);
                }
            }
        }
    }
    false
}

pub(crate) fn local_repo_candidates(project_dtag: &str, clone_url: Option<&str>) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(candidate) = clone_url.and_then(clone_url_owner_repo_name) {
        candidates.push(candidate);
    }
    if let Some(candidate) = local_repo_name_candidate(project_dtag) {
        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
    if let Some(candidate) = clone_url.and_then(clone_url_repo_name) {
        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

pub(crate) fn find_local_repo_dir(
    repos_dir: Option<&str>,
    project_dtag: &str,
    clone_url: Option<&str>,
) -> Result<Option<std::path::PathBuf>, String> {
    let repos_roots = canonical_repos_roots(repos_dir)?;

    for repos_root in repos_roots {
        for candidate in local_repo_candidates(project_dtag, clone_url) {
            let candidate_path = repos_root.join(candidate);
            let Ok(candidate_path) = candidate_path.canonicalize() else {
                continue;
            };
            if !candidate_path.starts_with(&repos_root) || !candidate_path.is_dir() {
                continue;
            }
            if candidate_path.join(".git").exists()
                && clone_url
                    .map(|url| checkout_origin_matches(&candidate_path, &repos_root, url))
                    .unwrap_or(true)
            {
                return Ok(Some(candidate_path));
            }
        }
    }
    Ok(None)
}

pub(crate) fn default_repos_root_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(nest_dir().map(|path| path.join("REPOS")));
    candidates.extend(
        dirs::home_dir()
            .map(|home| home.join(".buzz").join("REPOS"))
            .filter(|path| !candidates.iter().any(|candidate| candidate == path)),
    );
    candidates
}

pub(crate) fn canonicalize_repos_root(
    repos_root: std::path::PathBuf,
) -> Result<std::path::PathBuf, String> {
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

pub(crate) fn canonical_repos_roots(
    repos_dir: Option<&str>,
) -> Result<Vec<std::path::PathBuf>, String> {
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
