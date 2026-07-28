use crate::client::BuzzClient;
use crate::commands::with_git_provenance;
use crate::error::CliError;
use crate::validate::{read_or_stdin, sdk_err, validate_hex64, validate_repo_id};
use buzz_core::kind::KIND_GIT_ISSUE_ASSIGNEE;
use buzz_sdk::{build_git_issue_assignment, GitIssueMeta, GitRepoCoord, GitStatusMeta};
use nostr::{Event, Timestamp};

fn parse_events(json: &str) -> Result<Vec<Event>, CliError> {
    serde_json::from_str(json)
        .map_err(|error| CliError::Other(format!("failed to parse relay response: {error}")))
}

fn monotonic_assignment_created_at(now: u64, prior_head: Option<u64>) -> u64 {
    prior_head.map_or(now, |prior| now.max(prior.saturating_add(1)))
}

fn current_assignment_filter(author: &str, issue: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [KIND_GIT_ISSUE_ASSIGNEE],
        "authors": [author],
        "#d": [issue.to_ascii_lowercase()],
        "limit": 1,
    })
}

async fn fetch_current_issue_assignment(
    client: &BuzzClient,
    issue: &str,
) -> Result<Option<Event>, CliError> {
    let filter = current_assignment_filter(&client.keys().public_key().to_hex(), issue);
    let raw = client.query(&filter).await?;
    Ok(parse_events(&raw)?.into_iter().next())
}

#[derive(Debug, PartialEq, Eq)]
enum AssignmentWriteDisposition {
    Applied,
    Duplicate,
}

fn assignment_write_disposition(raw: &str) -> Result<AssignmentWriteDisposition, CliError> {
    let response: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| CliError::Other(format!("relay response is not JSON: {error} ({raw})")))?;
    let accepted = response
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let message = response
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !accepted {
        return Err(CliError::Other(format!("relay rejected event: {message}")));
    }
    if message == "duplicate" || message.starts_with("duplicate:") {
        return Ok(AssignmentWriteDisposition::Duplicate);
    }
    Ok(AssignmentWriteDisposition::Applied)
}

pub async fn cmd_create_issue(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    subject: &str,
    content: &str,
    labels: &[String],
    to: &[String],
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    let body = read_or_stdin(content)?;

    let meta = GitIssueMeta {
        labels: labels.to_vec(),
        recipients: to.to_vec(),
    };

    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };

    let builder = with_git_provenance(
        buzz_sdk::build_git_issue(&repo, subject, &body, &meta).map_err(sdk_err)?,
    )?;
    let event = client.sign_event(builder)?;
    let event_id = event.id.to_hex();
    let resp = client.submit_event(event).await?;
    // `link` renders as a rich preview card in Buzz Desktop when included in
    // a chat message — agents announce issues with it (see base_prompt.md).
    let link = crate::links::issue_link(&event_id, repo_owner, repo_id);
    crate::client::print_create_response(&resp, "link", &link);
    Ok(())
}

pub async fn cmd_get_issue(client: &BuzzClient, event: &str) -> Result<(), CliError> {
    validate_hex64(event)?;
    let filter = serde_json::json!({
        "kinds": [1621],
        "ids": [event]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_list_issues(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    author: Option<&str>,
    label: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;

    let a_value = format!("30617:{repo_owner}:{repo_id}");
    let mut filter = serde_json::json!({
        "kinds": [1621],
        "#a": [a_value]
    });

    if let Some(pk) = author {
        validate_hex64(pk)?;
        filter["authors"] = serde_json::json!([pk]);
    }
    if let Some(l) = label {
        filter["#t"] = serde_json::json!([l]);
    }
    if let Some(n) = limit {
        filter["limit"] = serde_json::json!(n);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_list_issue_assignments(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    assignee: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    if let Some(pubkey) = assignee {
        validate_hex64(pubkey)?;
    }

    let repo_owner = repo_owner.to_ascii_lowercase();
    let a_value = format!("30617:{repo_owner}:{repo_id}");
    let mut filter = serde_json::json!({
        "kinds": [KIND_GIT_ISSUE_ASSIGNEE],
        "#a": [a_value]
    });
    if let Some(pubkey) = assignee {
        filter["#p"] = serde_json::json!([pubkey.to_ascii_lowercase()]);
    }
    if let Some(limit) = limit {
        filter["limit"] = serde_json::json!(limit);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_assign_issue(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    assignee: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(issue)?;
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    if let Some(pubkey) = assignee {
        validate_hex64(pubkey)?;
    }

    let issue = issue.to_ascii_lowercase();
    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };
    let prior = fetch_current_issue_assignment(client, &issue).await?;
    let created_at = monotonic_assignment_created_at(
        Timestamp::now().as_secs(),
        prior.map(|event| event.created_at.as_secs()),
    );
    let builder = build_git_issue_assignment(&repo, &issue, assignee)
        .map_err(sdk_err)?
        .custom_created_at(Timestamp::from(created_at));
    let event = client.sign_event(builder)?;
    let submitted_id = event.id.to_hex();
    let resp = client.submit_event(event).await?;
    match assignment_write_disposition(&resp)? {
        AssignmentWriteDisposition::Applied => println!("{resp}"),
        AssignmentWriteDisposition::Duplicate => {
            let current = fetch_current_issue_assignment(client, &issue)
                .await
                .map_err(|error| {
                    CliError::DeliveryUnknown(format!(
                        "assignment may have succeeded, but its current head could not be verified: {error}"
                    ))
                })?;
            if current.as_ref().map(|event| event.id.to_hex()).as_deref()
                != Some(submitted_id.as_str())
            {
                return Err(CliError::Conflict(
                    "assignment changed concurrently; fetch the current assignment and retry"
                        .into(),
                ));
            }
            println!(
                "{}",
                serde_json::json!({
                    "event_id": submitted_id,
                    "accepted": true,
                    "message": "idempotent: assignment already current",
                })
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_issue_status(
    client: &BuzzClient,
    issue: &str,
    status: &str,
    content: Option<&str>,
    repo_owner: Option<&str>,
    repo_id: Option<&str>,
    euc: Option<&str>,
    to: &[String],
) -> Result<(), CliError> {
    validate_hex64(issue)?;
    let status = crate::commands::patches::parse_status(status)?;
    let body = match content {
        Some(c) => read_or_stdin(c)?,
        None => String::new(),
    };

    let repo = match (repo_owner, repo_id) {
        (Some(owner), Some(id)) => {
            validate_hex64(owner)?;
            validate_repo_id(id)?;
            Some(GitRepoCoord {
                owner: owner.to_string(),
                id: id.to_string(),
            })
        }
        (None, None) => None,
        _ => {
            return Err(CliError::Usage(
                "--repo-owner and --repo-id must be given together".into(),
            ))
        }
    };

    // Mirrors `buzz patches status`: default a `p` tag to the repo owner
    // for discoverability, plus a `--to` escape hatch for the issue author
    // or anyone else who should be notified of the status change.
    let mut recipients = Vec::new();
    if let Some(ref repo) = repo {
        recipients.push(repo.owner.clone());
    }
    for recipient in to {
        validate_hex64(recipient)?;
        if !recipients.contains(recipient) {
            recipients.push(recipient.clone());
        }
    }

    let meta = GitStatusMeta {
        root_event: issue.to_string(),
        accepted_revision_root: None,
        repo,
        euc: euc.map(str::to_string),
        recipients,
        applied_patches: vec![],
        merge_commit: None,
        applied_as_commits: vec![],
    };

    let builder =
        with_git_provenance(buzz_sdk::build_git_status(status, &body, &meta).map_err(sdk_err)?)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn dispatch(cmd: crate::IssuesCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::IssuesCmd;
    match cmd {
        IssuesCmd::Create {
            repo_owner,
            repo_id,
            title,
            content,
            label,
            to,
        } => cmd_create_issue(client, &repo_owner, &repo_id, &title, &content, &label, &to).await,
        IssuesCmd::Get { event } => cmd_get_issue(client, &event).await,
        IssuesCmd::List {
            repo_owner,
            repo_id,
            author,
            label,
            limit,
        } => {
            cmd_list_issues(
                client,
                &repo_owner,
                &repo_id,
                author.as_deref(),
                label.as_deref(),
                limit,
            )
            .await
        }
        IssuesCmd::Assign {
            issue,
            repo_owner,
            repo_id,
            assignee,
            unassign,
        } => {
            debug_assert_eq!(unassign, assignee.is_none());
            cmd_assign_issue(client, &issue, &repo_owner, &repo_id, assignee.as_deref()).await
        }
        IssuesCmd::Assignments {
            repo_owner,
            repo_id,
            assignee,
            limit,
        } => {
            cmd_list_issue_assignments(client, &repo_owner, &repo_id, assignee.as_deref(), limit)
                .await
        }
        IssuesCmd::Status {
            issue,
            status,
            content,
            repo_owner,
            repo_id,
            euc,
            to,
        } => {
            cmd_issue_status(
                client,
                &issue,
                &status,
                content.as_deref(),
                repo_owner.as_deref(),
                repo_id.as_deref(),
                euc.as_deref(),
                &to,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assignment_write_disposition, current_assignment_filter, monotonic_assignment_created_at,
        AssignmentWriteDisposition,
    };

    #[test]
    fn assignment_writes_advance_past_same_second_and_future_heads() {
        assert_eq!(monotonic_assignment_created_at(100, None), 100);
        assert_eq!(monotonic_assignment_created_at(100, Some(50)), 100);
        assert_eq!(monotonic_assignment_created_at(100, Some(100)), 101);
        assert_eq!(monotonic_assignment_created_at(100, Some(200)), 201);
    }

    #[test]
    fn duplicate_assignment_write_requires_head_verification() {
        let disposition = assignment_write_disposition(
            r#"{"event_id":"abc","accepted":true,"message":"duplicate: superseded"}"#,
        )
        .unwrap();
        assert_eq!(disposition, AssignmentWriteDisposition::Duplicate);
    }

    #[test]
    fn assignment_head_filter_normalizes_the_replacement_key() {
        let filter = current_assignment_filter(&"a".repeat(64), &"E".repeat(64));
        assert_eq!(filter["#d"], serde_json::json!(["e".repeat(64)]));
    }
}
