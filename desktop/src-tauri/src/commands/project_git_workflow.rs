//! Local clone and pull-request merge commands for the Projects workflow.

use super::project_git::{first_output_line, normalize_branch_option};
use super::project_git_diff::clean_commit;
use super::project_git_exec::{
    build_git_auth_config, build_git_auth_config_for_keys, clone_url_owner, run_git,
    validate_clone_url, validate_workspace_clone_url, GitAuthConfig,
};
use super::project_repo_paths::{
    canonical_repos_roots, canonicalize_repos_root, default_repos_root_candidates,
    find_local_repo_dir, local_repo_candidates,
};
use crate::app_state::AppState;
use crate::managed_agents::{load_managed_agents, spawn_key_refusal};
use crate::relay::submit_signed_event_with_keys;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

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
    pub status_event: String,
    pub status_publication_error: Option<String>,
}

/// Machine-readable recovery metadata for a failed pull-request merge.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPullRequestMergeRecovery {
    action: String,
    target_branch: String,
    source_branch: String,
}

/// Structured pull-request merge failure returned across the Tauri boundary.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPullRequestMergeError {
    code: String,
    message: String,
    recovery: Option<ProjectPullRequestMergeRecovery>,
}

impl ProjectPullRequestMergeError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            recovery: None,
        }
    }

    fn conflict(target_branch: String, source_branch: String) -> Self {
        Self {
            code: "merge_conflict".to_string(),
            message: "Pull request has merge conflicts.".to_string(),
            recovery: Some(ProjectPullRequestMergeRecovery {
                action: "open_terminal".to_string(),
                target_branch,
                source_branch,
            }),
        }
    }
}

impl From<String> for ProjectPullRequestMergeError {
    fn from(message: String) -> Self {
        Self::new("merge_failed", message)
    }
}

fn classify_merge_error(
    message: String,
    has_conflicts: bool,
    target_branch: &str,
    source_branch: &str,
) -> ProjectPullRequestMergeError {
    if has_conflicts {
        ProjectPullRequestMergeError::conflict(target_branch.to_string(), source_branch.to_string())
    } else {
        ProjectPullRequestMergeError::new(
            "merge_failed",
            format!("Pull request merge failed: {message}"),
        )
    }
}

struct ProjectRepoMergeGitResult {
    message: String,
    merge_commit: String,
}

/// Validated repository and pull-request metadata for a native merge.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPullRequestMergeInput {
    target_clone_url: String,
    source_clone_url: String,
    target_owner: String,
    repo_address: String,
    pull_request_id: String,
    pull_request_author: String,
    status_created_at: u64,
    target_branch: String,
    source_branch: String,
    expected_commit: String,
}

/// Repository-scoped metadata for an agent-signed review request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPullRequestReviewRequestInput {
    target_owner: String,
    repo_address: String,
    pull_request_id: String,
    reviewers: Vec<String>,
    reviewer_label: String,
}

/// A previously signed merged-status event that needs publishing again.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPullRequestMergedStatusInput {
    target_owner: String,
    status_event: String,
}

fn normalize_commit(value: &str) -> Option<String> {
    clean_commit(Some(value.trim().to_ascii_lowercase()))
}

fn normalize_event_id(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    (value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())).then_some(value)
}

struct ProjectOwnerIdentity {
    keys: Keys,
    auth_tag: Option<String>,
}

fn project_owner_identity(
    app: &AppHandle,
    state: &AppState,
    target_owner: &str,
) -> Result<ProjectOwnerIdentity, String> {
    let viewer_keys = state.signing_keys()?;
    if viewer_keys.public_key().to_hex() == target_owner {
        return Ok(ProjectOwnerIdentity {
            keys: viewer_keys,
            auth_tag: None,
        });
    }

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let records = load_managed_agents(app)?;
    let record = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(target_owner))
        .ok_or_else(|| {
            "Only the repository owner or the owner of its managed agent can merge pull requests."
                .to_string()
        })?;
    if let Some(error) = spawn_key_refusal(record) {
        return Err(error);
    }
    let keys = Keys::parse(&record.private_key_nsec)
        .map_err(|error| format!("managed agent signing key is invalid: {error}"))?;
    if keys.public_key().to_hex() != target_owner {
        return Err("Managed agent key does not match the repository owner.".to_string());
    }
    Ok(ProjectOwnerIdentity {
        keys,
        auth_tag: record.auth_tag.clone(),
    })
}

fn validate_repo_address(repo_address: &str, owner: &str) -> Result<(), String> {
    let prefix = format!("30617:{owner}:");
    if repo_address.strip_prefix(&prefix).is_none_or(str::is_empty) {
        return Err("Repository address does not match the repository owner.".to_string());
    }
    Ok(())
}

fn validate_merge_status_metadata(
    repo_address: &str,
    owner: &str,
    pull_request_id: &str,
    pull_request_author: &str,
) -> Result<(String, String), String> {
    validate_repo_address(repo_address, owner)?;
    let pull_request_id = normalize_event_id(pull_request_id)
        .ok_or_else(|| "Invalid pull request event ID.".to_string())?;
    let pull_request_author = normalize_event_id(pull_request_author)
        .ok_or_else(|| "Invalid pull request author.".to_string())?;
    Ok((pull_request_id, pull_request_author))
}

fn build_merged_status_event(
    keys: &Keys,
    repo_address: &str,
    pull_request_id: &str,
    pull_request_author: &str,
    merge_commit: &str,
    created_at: u64,
) -> Result<String, String> {
    let owner = keys.public_key().to_hex();
    let (pull_request_id, pull_request_author) =
        validate_merge_status_metadata(repo_address, &owner, pull_request_id, pull_request_author)?;
    let merge_commit =
        normalize_commit(merge_commit).ok_or_else(|| "Invalid merge commit.".to_string())?;
    let created_at = created_at.max(Timestamp::now().as_secs());

    let mut raw_tags = vec![
        vec!["e", pull_request_id.as_str(), "", "root"],
        vec!["a", repo_address],
        vec!["p", owner.as_str()],
    ];
    if pull_request_author != owner {
        raw_tags.push(vec!["p", pull_request_author.as_str()]);
    }
    raw_tags.extend([
        vec!["merge-commit", merge_commit.as_str()],
        vec!["r", merge_commit.as_str()],
    ]);
    let tags = raw_tags
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("build merged status tags: {error}"))?;
    EventBuilder::new(Kind::Custom(1631), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .map(|event| event.as_json())
        .map_err(|error| format!("sign merged pull request status: {error}"))
}

fn build_review_request_event(
    keys: &Keys,
    repo_address: &str,
    pull_request_id: &str,
    reviewers: &[String],
    reviewer_label: &str,
) -> Result<String, String> {
    let owner = keys.public_key().to_hex();
    validate_repo_address(repo_address, &owner)?;
    let pull_request_id = normalize_event_id(pull_request_id)
        .ok_or_else(|| "Invalid pull request event ID.".to_string())?;
    if reviewers.is_empty() || reviewers.len() > 50 {
        return Err("Select between 1 and 50 reviewers.".to_string());
    }
    let mut reviewers = reviewers
        .iter()
        .map(|reviewer| {
            normalize_event_id(reviewer).ok_or_else(|| "Invalid reviewer pubkey.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    reviewers.sort();
    reviewers.dedup();
    let reviewer_label = reviewer_label.trim();
    if reviewer_label.is_empty() || reviewer_label.chars().count() > 128 {
        return Err("Reviewer label must be between 1 and 128 characters.".to_string());
    }

    let mut raw_tags = vec![
        vec![
            "e".to_string(),
            pull_request_id,
            String::new(),
            "root".to_string(),
        ],
        vec!["a".to_string(), repo_address.to_string()],
    ];
    raw_tags.extend(
        reviewers
            .into_iter()
            .map(|reviewer| vec!["p".to_string(), reviewer]),
    );
    raw_tags.push(vec!["t".to_string(), "review-request".to_string()]);
    let tags = raw_tags
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("build review request tags: {error}"))?;
    EventBuilder::new(
        Kind::TextNote,
        format!("Requested a review from {reviewer_label}"),
    )
    .tags(tags)
    .sign_with_keys(keys)
    .map(|event| event.as_json())
    .map_err(|error| format!("sign pull request review request: {error}"))
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
pub async fn sign_project_pull_request_review_request(
    input: ProjectPullRequestReviewRequestInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let target_owner = input.target_owner.trim().to_ascii_lowercase();
    if normalize_event_id(&target_owner).is_none() {
        return Err("Invalid target repository owner.".to_string());
    }
    let identity = project_owner_identity(&app, &state, &target_owner)?;
    let event = Event::from_json(build_review_request_event(
        &identity.keys,
        &input.repo_address,
        &input.pull_request_id,
        &input.reviewers,
        &input.reviewer_label,
    )?)
    .map_err(|error| format!("parse signed review request: {error}"))?;
    submit_signed_event_with_keys(&event, &state, &identity.keys, identity.auth_tag.as_deref())
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn publish_project_pull_request_merged_status(
    input: ProjectPullRequestMergedStatusInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let target_owner = input.target_owner.trim().to_ascii_lowercase();
    if normalize_event_id(&target_owner).is_none() {
        return Err("Invalid target repository owner.".to_string());
    }
    let event = Event::from_json(input.status_event)
        .map_err(|error| format!("parse merged status event: {error}"))?;
    if event.kind != Kind::Custom(1631)
        || event.pubkey.to_hex() != target_owner
        || event.verify().is_err()
    {
        return Err("Invalid merged pull request status event.".to_string());
    }
    let identity = project_owner_identity(&app, &state, &target_owner)?;
    submit_signed_event_with_keys(&event, &state, &identity.keys, identity.auth_tag.as_deref())
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn merge_project_pull_request(
    input: ProjectPullRequestMergeInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ProjectRepoMergeResult, ProjectPullRequestMergeError> {
    let ProjectPullRequestMergeInput {
        target_clone_url,
        source_clone_url,
        target_owner,
        repo_address,
        pull_request_id,
        pull_request_author,
        status_created_at,
        target_branch,
        source_branch,
        expected_commit,
    } = input;
    validate_workspace_clone_url(&target_clone_url, &state)?;
    validate_workspace_clone_url(&source_clone_url, &state)?;
    let target_owner = target_owner.trim().to_ascii_lowercase();
    if target_owner.len() != 64 || !target_owner.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid target repository owner.".to_string().into());
    }
    if clone_url_owner(&target_clone_url).as_deref() != Some(target_owner.as_str()) {
        return Err("Target clone URL does not match the repository owner."
            .to_string()
            .into());
    }
    let owner_identity = project_owner_identity(&app, &state, &target_owner)?;
    let merger_pubkey = owner_identity.keys.public_key().to_hex();
    let target_branch = normalize_branch_option(Some(&target_branch))
        .ok_or_else(|| "Invalid target branch.".to_string())?;
    let source_branch = normalize_branch_option(Some(&source_branch))
        .ok_or_else(|| "Invalid source branch.".to_string())?;
    if target_branch == source_branch && same_repository(&target_clone_url, &source_clone_url) {
        return Err("Source and target branches must be different."
            .to_string()
            .into());
    }
    let expected_commit = normalize_commit(&expected_commit)
        .ok_or_else(|| "Invalid pull request commit.".to_string())?;
    let (pull_request_id, pull_request_author) = validate_merge_status_metadata(
        &repo_address,
        &merger_pubkey,
        &pull_request_id,
        &pull_request_author,
    )?;
    let auth = build_git_auth_config_for_keys(&owner_identity.keys)?;

    let git_result = tauri::async_runtime::spawn_blocking(
        move || -> Result<ProjectRepoMergeGitResult, ProjectPullRequestMergeError> {
            let temp_dir =
                tempfile::tempdir().map_err(|error| format!("create temp dir: {error}"))?;
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
                return Err(ProjectPullRequestMergeError::new(
                    "branch_changed",
                    "The pull request branch changed. Refresh the pull request before merging."
                        .to_string(),
                ));
            }

            let merge_email = format!("{merger_pubkey}@users.noreply.buzz");
            let merge_result = run_git(
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
            );
            if let Err(error) = merge_result {
                let has_conflicts = run_git(
                    &["diff", "--name-only", "--diff-filter=U"],
                    Some(&repo_dir),
                    &auth,
                )
                .is_ok_and(|output| !output.trim().is_empty());
                return Err(classify_merge_error(
                    error,
                    has_conflicts,
                    &target_branch,
                    &source_branch,
                ));
            }
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

            Ok(ProjectRepoMergeGitResult {
                message: format!("Merged {source_branch} into {target_branch}."),
                merge_commit,
            })
        },
    )
    .await
    .map_err(|error| {
        ProjectPullRequestMergeError::new(
            "merge_task_failed",
            format!("pull request merge task failed: {error}"),
        )
    })??;
    let status_event = build_merged_status_event(
        &owner_identity.keys,
        &repo_address,
        &pull_request_id,
        &pull_request_author,
        &git_result.merge_commit,
        status_created_at,
    )?;
    let signed_status = Event::from_json(&status_event)
        .map_err(|error| format!("parse signed merged status: {error}"))?;
    let status_publication_error = submit_signed_event_with_keys(
        &signed_status,
        &state,
        &owner_identity.keys,
        owner_identity.auth_tag.as_deref(),
    )
    .await
    .err();
    Ok(ProjectRepoMergeResult {
        message: git_result.message,
        merge_commit: git_result.merge_commit,
        status_event,
        status_publication_error,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_merged_status_event, build_review_request_event, classify_merge_error,
        normalize_commit, same_repository, validate_merge_status_metadata,
        ProjectPullRequestMergeError,
    };
    use nostr::{Event, JsonUtil, Keys, Timestamp};

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

    #[test]
    fn merge_conflict_error_has_stable_recovery_metadata() {
        let error =
            ProjectPullRequestMergeError::conflict("main".to_string(), "feature/demo".to_string());

        assert_eq!(error.code, "merge_conflict");
        assert_eq!(error.message, "Pull request has merge conflicts.");
        let recovery = error.recovery.expect("conflict recovery");
        assert_eq!(recovery.action, "open_terminal");
        assert_eq!(recovery.target_branch, "main");
        assert_eq!(recovery.source_branch, "feature/demo");
    }

    #[test]
    fn merge_conflict_error_serializes_for_tauri_clients() {
        let error =
            ProjectPullRequestMergeError::conflict("main".to_string(), "feature/demo".to_string());
        let value = serde_json::to_value(error).expect("serialize merge conflict");

        assert_eq!(value["code"], "merge_conflict");
        assert_eq!(value["recovery"]["targetBranch"], "main");
        assert_eq!(value["recovery"]["sourceBranch"], "feature/demo");
    }

    #[test]
    fn merge_error_classification_only_recovers_conflicts() {
        let conflict = classify_merge_error(
            "CONFLICT (content): Merge conflict in src/main.rs".to_string(),
            true,
            "main",
            "feature/demo",
        );
        assert_eq!(conflict.code, "merge_conflict");
        assert!(conflict.recovery.is_some());

        let other = classify_merge_error(
            "fatal: refusing to merge unrelated histories".to_string(),
            false,
            "main",
            "feature/demo",
        );
        assert_eq!(other.code, "merge_failed");
        assert!(other.recovery.is_none());
    }

    #[test]
    fn merged_status_is_signed_by_repository_owner() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let pull_request_id = "d".repeat(64);
        let pull_request_author = "b".repeat(64);
        let merge_commit = "e".repeat(40);
        let repo_address = format!("30617:{owner}:buzz");
        let before = Timestamp::now().as_secs();
        let event = Event::from_json(
            build_merged_status_event(
                &keys,
                &repo_address,
                &pull_request_id,
                &pull_request_author,
                &merge_commit,
                123,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(event.kind.as_u16(), 1631);
        assert!(event.created_at.as_secs() >= before);
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["merge-commit", merge_commit.as_str()]));
        assert!(event.verify().is_ok());
    }

    #[test]
    fn merged_status_preserves_a_newer_requested_timestamp() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let requested = Timestamp::now().as_secs() + 10;
        let event = Event::from_json(
            build_merged_status_event(
                &keys,
                &format!("30617:{owner}:buzz"),
                &"d".repeat(64),
                &"b".repeat(64),
                &"e".repeat(40),
                requested,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(event.created_at.as_secs(), requested);
    }

    #[test]
    fn merge_status_metadata_is_rejected_before_git_work() {
        let owner = "a".repeat(64);
        assert!(validate_merge_status_metadata(
            &format!("30617:{}:buzz", "b".repeat(64)),
            &owner,
            &"d".repeat(64),
            &"e".repeat(64),
        )
        .is_err());
        assert!(validate_merge_status_metadata(
            &format!("30617:{owner}:buzz"),
            &owner,
            "not-an-event-id",
            &"e".repeat(64),
        )
        .is_err());
        assert!(validate_merge_status_metadata(
            &format!("30617:{owner}:buzz"),
            &owner,
            &"d".repeat(64),
            "not-an-author",
        )
        .is_err());
    }

    #[test]
    fn review_request_is_signed_by_repository_owner() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let reviewer = "b".repeat(64);
        let repo_address = format!("30617:{owner}:buzz");
        let event = Event::from_json(
            build_review_request_event(
                &keys,
                &repo_address,
                &"d".repeat(64),
                std::slice::from_ref(&reviewer),
                "Bob",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(event.kind, nostr::Kind::TextNote);
        assert_eq!(event.content, "Requested a review from Bob");
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["p", reviewer.as_str()]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["t", "review-request"]));
        assert!(event.verify().is_ok());
    }
}
