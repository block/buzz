//! Labeled recipient notes for the Projects workflow: kind:1 comments whose
//! `p` tags name recipients on a root event. Pull-request review requests
//! (`t: review-request`) and issue assignments (`t: assignment`) share this
//! shape so clients can parse them with one code path.

use super::project_git_workflow::{
    normalize_event_id, project_owner_identity, validate_repo_address,
};
use crate::app_state::AppState;
use crate::relay::submit_signed_event_with_keys;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde::Deserialize;
use tauri::{AppHandle, State};

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

/// Repository-scoped metadata for an agent-signed issue assignee operation.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIssueAssigneeOperationInput {
    target_owner: String,
    repo_address: String,
    issue_id: String,
    assignees: Vec<String>,
    assignee_label: String,
    created_at: u64,
}

#[derive(Clone, Copy)]
enum IssueAssigneeOperation {
    Assign,
    Unassign,
}

impl IssueAssigneeOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Assign => "assignment",
            Self::Unassign => "unassignment",
        }
    }

    fn content(self, assignee_label: &str) -> String {
        match self {
            Self::Assign => format!("Assigned this issue to {assignee_label}"),
            Self::Unassign => format!("Unassigned {assignee_label} from this issue"),
        }
    }
}

/// Parameters for [`build_labeled_recipient_note_event`].
struct LabeledRecipientNote<'a> {
    repo_address: &'a str,
    root_id: &'a str,
    root_id_error: &'a str,
    recipients: &'a [String],
    recipient_noun: &'a str,
    label: &'a str,
    content: String,
    created_at: Option<u64>,
}

/// Shared builder for labeled kind:1 notes tagging recipients (`p`) on a
/// root event — the convention used by both PR review requests
/// (`t: review-request`) and issue assignments (`t: assignment`).
fn build_labeled_recipient_note_event(
    keys: &Keys,
    note: LabeledRecipientNote<'_>,
) -> Result<String, String> {
    let LabeledRecipientNote {
        repo_address,
        root_id,
        root_id_error,
        recipients,
        recipient_noun,
        label,
        content,
        created_at,
    } = note;
    let owner = keys.public_key().to_hex();
    validate_repo_address(repo_address, &owner)?;
    let root_id = normalize_event_id(root_id).ok_or_else(|| root_id_error.to_string())?;
    if recipients.is_empty() || recipients.len() > 50 {
        return Err(format!("Select between 1 and 50 {recipient_noun}s."));
    }
    let mut recipients = recipients
        .iter()
        .map(|recipient| {
            normalize_event_id(recipient).ok_or_else(|| format!("Invalid {recipient_noun} pubkey."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    recipients.sort();
    recipients.dedup();

    let mut raw_tags = vec![
        vec!["e".to_string(), root_id, String::new(), "root".to_string()],
        vec!["a".to_string(), repo_address.to_string()],
    ];
    raw_tags.extend(
        recipients
            .into_iter()
            .map(|recipient| vec!["p".to_string(), recipient]),
    );
    raw_tags.push(vec!["t".to_string(), label.to_string()]);
    let tags = raw_tags
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("build {label} tags: {error}"))?;
    let mut builder = EventBuilder::new(Kind::TextNote, content).tags(tags);
    if let Some(created_at) = created_at {
        builder = builder.custom_created_at(Timestamp::from_secs(created_at));
    }
    builder
        .sign_with_keys(keys)
        .map(|event| event.as_json())
        .map_err(|error| format!("sign {label} note: {error}"))
}

fn build_review_request_event(
    keys: &Keys,
    repo_address: &str,
    pull_request_id: &str,
    reviewers: &[String],
    reviewer_label: &str,
) -> Result<String, String> {
    let reviewer_label = reviewer_label.trim();
    if reviewer_label.is_empty() || reviewer_label.chars().count() > 128 {
        return Err("Reviewer label must be between 1 and 128 characters.".to_string());
    }
    build_labeled_recipient_note_event(
        keys,
        LabeledRecipientNote {
            repo_address,
            root_id: pull_request_id,
            root_id_error: "Invalid pull request event ID.",
            recipients: reviewers,
            recipient_noun: "reviewer",
            label: "review-request",
            content: format!("Requested a review from {reviewer_label}"),
            created_at: None,
        },
    )
}

#[cfg(test)]
fn build_issue_assignment_event(
    keys: &Keys,
    repo_address: &str,
    issue_id: &str,
    assignees: &[String],
    assignee_label: &str,
    created_at: Option<u64>,
) -> Result<String, String> {
    build_issue_assignee_operation_event(
        keys,
        repo_address,
        issue_id,
        assignees,
        assignee_label,
        created_at,
        IssueAssigneeOperation::Assign,
    )
}

#[cfg(test)]
fn build_issue_unassignment_event(
    keys: &Keys,
    repo_address: &str,
    issue_id: &str,
    assignees: &[String],
    assignee_label: &str,
    created_at: Option<u64>,
) -> Result<String, String> {
    build_issue_assignee_operation_event(
        keys,
        repo_address,
        issue_id,
        assignees,
        assignee_label,
        created_at,
        IssueAssigneeOperation::Unassign,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_issue_assignee_operation_event(
    keys: &Keys,
    repo_address: &str,
    issue_id: &str,
    assignees: &[String],
    assignee_label: &str,
    created_at: Option<u64>,
    operation: IssueAssigneeOperation,
) -> Result<String, String> {
    let assignee_label = assignee_label.trim();
    if assignee_label.is_empty() || assignee_label.chars().count() > 128 {
        return Err("Assignee label must be between 1 and 128 characters.".to_string());
    }
    build_labeled_recipient_note_event(
        keys,
        LabeledRecipientNote {
            repo_address,
            root_id: issue_id,
            root_id_error: "Invalid issue event ID.",
            recipients: assignees,
            recipient_noun: "assignee",
            label: operation.label(),
            content: operation.content(assignee_label),
            created_at,
        },
    )
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
pub async fn sign_project_issue_assignment(
    input: ProjectIssueAssigneeOperationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    sign_project_issue_assignee_operation(input, IssueAssigneeOperation::Assign, app, state).await
}

#[tauri::command]
pub async fn sign_project_issue_unassignment(
    input: ProjectIssueAssigneeOperationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    sign_project_issue_assignee_operation(input, IssueAssigneeOperation::Unassign, app, state).await
}

async fn sign_project_issue_assignee_operation(
    input: ProjectIssueAssigneeOperationInput,
    operation: IssueAssigneeOperation,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let target_owner = input.target_owner.trim().to_ascii_lowercase();
    if normalize_event_id(&target_owner).is_none() {
        return Err("Invalid target repository owner.".to_string());
    }
    let identity = project_owner_identity(&app, &state, &target_owner)?;
    let event = Event::from_json(build_issue_assignee_operation_event(
        &identity.keys,
        &input.repo_address,
        &input.issue_id,
        &input.assignees,
        &input.assignee_label,
        Some(input.created_at),
        operation,
    )?)
    .map_err(|error| format!("parse signed issue {}: {error}", operation.label()))?;
    submit_signed_event_with_keys(&event, &state, &identity.keys, identity.auth_tag.as_deref())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_issue_assignment_event, build_issue_unassignment_event, build_review_request_event,
    };
    use nostr::{Event, JsonUtil, Keys};

    #[test]
    fn issue_assignment_is_signed_by_repository_owner() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let assignee = "b".repeat(64);
        let repo_address = format!("30617:{owner}:buzz");
        let event = Event::from_json(
            build_issue_assignment_event(
                &keys,
                &repo_address,
                &"d".repeat(64),
                std::slice::from_ref(&assignee),
                "Bob",
                None,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(event.kind, nostr::Kind::TextNote);
        assert_eq!(event.content, "Assigned this issue to Bob");
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["p", assignee.as_str()]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["t", "assignment"]));
        assert!(event.verify().is_ok());
    }

    #[test]
    fn issue_assignment_rejects_invalid_metadata() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let repo_address = format!("30617:{owner}:buzz");

        assert!(build_issue_assignment_event(
            &keys,
            &repo_address,
            &"d".repeat(64),
            &[],
            "Bob",
            None,
        )
        .is_err());
        assert!(build_issue_assignment_event(
            &keys,
            &repo_address,
            &"d".repeat(64),
            &["b".repeat(64)],
            "  ",
            None,
        )
        .is_err());
        assert!(build_issue_assignment_event(
            &keys,
            &repo_address,
            "not-an-event-id",
            &["b".repeat(64)],
            "Bob",
            None,
        )
        .is_err());
    }

    #[test]
    fn issue_unassignment_is_signed_by_repository_owner() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let assignee = "b".repeat(64);
        let repo_address = format!("30617:{owner}:buzz");
        let event = Event::from_json(
            build_issue_unassignment_event(
                &keys,
                &repo_address,
                &"d".repeat(64),
                std::slice::from_ref(&assignee),
                "Bob",
                Some(123),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(event.content, "Unassigned Bob from this issue");
        assert_eq!(event.created_at.as_secs(), 123);
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["p", assignee.as_str()]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["t", "unassignment"]));
        assert!(event.verify().is_ok());
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
