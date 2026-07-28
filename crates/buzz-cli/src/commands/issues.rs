use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::{read_or_stdin, sdk_err, validate_hex64, validate_repo_id};
use buzz_sdk::{GitIssueMeta, GitRepoCoord, GitStatusMeta};

fn issue_comment_recipients(
    raw: &str,
    issue_id: &str,
    repo: &GitRepoCoord,
    additional: &[String],
) -> Result<Vec<String>, CliError> {
    let events: Vec<serde_json::Value> = serde_json::from_str(raw)
        .map_err(|error| CliError::Other(format!("failed to parse relay response: {error}")))?;
    let issue = events
        .iter()
        .find(|event| {
            event.get("kind").and_then(serde_json::Value::as_u64) == Some(1621)
                && event
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| id.eq_ignore_ascii_case(issue_id))
        })
        .ok_or_else(|| CliError::NotFound(format!("issue not found: {issue_id}")))?;
    let tags = issue
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::Other("issue event is missing tags".into()))?;
    let repo_address = format!("30617:{}:{}", repo.owner.to_ascii_lowercase(), repo.id);
    let belongs_to_repo = tags.iter().any(|tag| {
        tag.as_array().is_some_and(|values| {
            values.first().and_then(serde_json::Value::as_str) == Some("a")
                && values.get(1).and_then(serde_json::Value::as_str) == Some(repo_address.as_str())
        })
    });
    if !belongs_to_repo {
        return Err(CliError::Usage(format!(
            "issue {issue_id} does not belong to repository {repo_address}"
        )));
    }

    let author = issue
        .get("pubkey")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::Other("issue event is missing its author pubkey".into()))?;
    validate_hex64(author)
        .map_err(|_| CliError::Other("issue event has an invalid author pubkey".into()))?;

    let mut recipients = vec![author.to_ascii_lowercase()];
    for tag in tags {
        let Some(values) = tag.as_array() else {
            continue;
        };
        if values.first().and_then(serde_json::Value::as_str) != Some("p") {
            continue;
        }
        let recipient = values
            .get(1)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CliError::Other("issue event contains an invalid p tag".into()))?;
        validate_hex64(recipient)
            .map_err(|_| CliError::Other("issue event contains an invalid p tag".into()))?;
        recipients.push(recipient.to_ascii_lowercase());
    }
    recipients.extend(additional.iter().cloned());
    Ok(recipients)
}

pub async fn cmd_comment_issue(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    content: &str,
    to: &[String],
) -> Result<(), CliError> {
    validate_hex64(issue)?;
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    for recipient in to {
        validate_hex64(recipient)?;
    }

    let body = read_or_stdin(content)?;
    let body = body.trim();
    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };
    let issue_id = issue.to_ascii_lowercase();
    let filter = serde_json::json!({
        "kinds": [1621],
        "ids": [issue_id],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let recipients = issue_comment_recipients(&raw, &issue_id, &repo, to)?;
    let builder =
        buzz_sdk::build_git_issue_comment(&repo, &issue_id, body, &recipients).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
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

    let builder = buzz_sdk::build_git_issue(&repo, subject, &body, &meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
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

    let builder = buzz_sdk::build_git_status(status, &body, &meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn dispatch(cmd: crate::IssuesCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::IssuesCmd;
    match cmd {
        IssuesCmd::Comment {
            issue,
            repo_owner,
            repo_id,
            content,
            to,
        } => cmd_comment_issue(client, &issue, &repo_owner, &repo_id, &content, &to).await,
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
    use super::*;

    fn issue_event_json(
        issue_id: &str,
        author: &str,
        repo_address: &str,
        recipients: &[&str],
    ) -> String {
        let mut tags = vec![serde_json::json!(["a", repo_address])];
        tags.extend(
            recipients
                .iter()
                .map(|recipient| serde_json::json!(["p", recipient])),
        );
        serde_json::json!([{
            "id": issue_id,
            "pubkey": author,
            "kind": 1621,
            "tags": tags,
        }])
        .to_string()
    }

    #[test]
    fn comment_recipients_include_issue_author_and_existing_recipients() {
        let owner = "a".repeat(64);
        let author = "b".repeat(64);
        let existing = "c".repeat(64);
        let additional = "d".repeat(64);
        let issue_id = "e".repeat(64);
        let repo = GitRepoCoord {
            owner: owner.clone(),
            id: "repo".into(),
        };
        let raw = issue_event_json(
            &issue_id,
            &author,
            &format!("30617:{owner}:repo"),
            &[&owner, &existing],
        );

        let recipients =
            issue_comment_recipients(&raw, &issue_id, &repo, std::slice::from_ref(&additional))
                .unwrap();

        assert_eq!(recipients, vec![author, owner, existing, additional]);
    }

    #[test]
    fn comment_recipients_reject_issue_from_another_repo() {
        let owner = "a".repeat(64);
        let issue_id = "e".repeat(64);
        let repo = GitRepoCoord {
            owner,
            id: "expected".into(),
        };
        let raw = issue_event_json(
            &issue_id,
            &"b".repeat(64),
            &format!("30617:{}:other", "a".repeat(64)),
            &[],
        );

        let error = issue_comment_recipients(&raw, &issue_id, &repo, &[]).unwrap_err();

        assert!(matches!(error, CliError::Usage(_)));
        assert!(error.to_string().contains("does not belong to repository"));
    }
}
