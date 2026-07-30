//! Structured pull-request merge failures returned across the Tauri boundary.

use serde::Serialize;

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
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
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
        // Relay push-policy denial for a repo with no `buzz-channel` binding.
        // The stable token is declared in `buzz-core::git_perms`
        // (GIT_NO_CHANNEL_BINDING_TOKEN); the relay guarantees the denial body
        // starts with it. Push failures reach this conversion as raw
        // stderr/`remote:` text, so match the token anywhere in the message.
        if message.contains(buzz_core_pkg::git_perms::GIT_NO_CHANNEL_BINDING_TOKEN) {
            return Self::new(
                buzz_core_pkg::git_perms::GIT_NO_CHANNEL_BINDING_TOKEN,
                "This repository is not bound to a channel, so the relay cannot \
                 authorize pushes. Bind it with: buzz repos bind --id <repo> \
                 --channel <channel-uuid>",
            );
        }
        Self::new("merge_failed", message)
    }
}

pub(crate) fn classify_merge_error(
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

#[cfg(test)]
mod tests {
    use super::{classify_merge_error, ProjectPullRequestMergeError};

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
    fn no_channel_binding_denial_converts_to_structured_code() {
        // The relay's push-policy denial arrives as raw git stderr with
        // `remote:` framing; the stable token must be recognized wherever it
        // sits in the message.
        let remote_stderr = format!(
            "remote: {}\nerror: failed to push some refs",
            buzz_core_pkg::git_perms::GIT_NO_CHANNEL_BINDING_BODY
        );
        let error = ProjectPullRequestMergeError::from(remote_stderr);

        assert_eq!(
            error.code,
            buzz_core_pkg::git_perms::GIT_NO_CHANNEL_BINDING_TOKEN
        );
        assert!(error.message.contains("buzz repos bind"));
        assert!(error.recovery.is_none());

        // Unrelated push failures keep the generic code and original text.
        let generic = ProjectPullRequestMergeError::from("connection reset".to_string());
        assert_eq!(generic.code, "merge_failed");
        assert_eq!(generic.message, "connection reset");
    }
}
