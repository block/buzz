use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::{
    read_file_or_stdin, read_or_stdin, sdk_err, validate_hex64, validate_repo_id,
};
use buzz_sdk::{
    GitDiffSide, GitPrCommentAnchor, GitPrCommentMeta, GitPrReviewDecision, GitPrUpdateMeta,
    GitPullRequestMeta, GitRepoCoord, GitStatusMeta,
};

fn read_optional_body(body: Option<&str>, body_file: Option<&str>) -> Result<String, CliError> {
    match (body, body_file) {
        (Some(_), Some(_)) => Err(CliError::Usage(
            "--body and --body-file are mutually exclusive".into(),
        )),
        (Some(value), None) => read_or_stdin(value),
        (None, Some(path)) => read_file_or_stdin(path),
        (None, None) => Ok(String::new()),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_open_pr(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    subject: &str,
    body: Option<&str>,
    body_file: Option<&str>,
    commit: &str,
    clone_urls: &[String],
    branch_name: Option<&str>,
    merge_base: Option<&str>,
    euc: Option<&str>,
    labels: &[String],
    to: &[String],
    channel: Option<&str>,
    revision_of: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    let content = read_optional_body(body, body_file)?;

    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };
    let meta = GitPullRequestMeta {
        euc: euc.map(str::to_string),
        recipients: to.to_vec(),
        channel_id: channel.map(str::to_string),
        subject: subject.to_string(),
        labels: labels.to_vec(),
        commit: commit.to_string(),
        clone_urls: clone_urls.to_vec(),
        branch_name: branch_name.map(str::to_string),
        merge_base: merge_base.map(str::to_string),
        revision_of: revision_of.map(str::to_string),
    };

    let builder = buzz_sdk::build_git_pull_request(&repo, &content, &meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_comment_pr(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    pr: &str,
    body: Option<&str>,
    body_file: Option<&str>,
    file: Option<&str>,
    line: Option<u32>,
    side: Option<&str>,
    commit: Option<&str>,
    to: &[String],
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    validate_hex64(pr)?;
    for recipient in to {
        validate_hex64(recipient)?;
    }
    if body.is_none() && body_file.is_none() {
        return Err(CliError::Usage(
            "a comment needs --body or --body-file".into(),
        ));
    }
    let content = read_optional_body(body, body_file)?;

    // clap's `requires_all` guarantees these arrive together.
    let anchor = match (file, line, side) {
        (Some(path), Some(line), Some(side)) => Some(GitPrCommentAnchor {
            path: path.to_string(),
            side: parse_diff_side(side)?,
            line,
        }),
        _ => None,
    };

    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };
    let meta = GitPrCommentMeta {
        recipients: to.to_vec(),
        anchor,
        commit: commit.map(str::to_string),
        decision: None,
    };

    let builder = buzz_sdk::build_git_pr_comment(&repo, pr, &content, &meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

fn parse_diff_side(side: &str) -> Result<GitDiffSide, CliError> {
    match side {
        "old" => Ok(GitDiffSide::Old),
        "new" => Ok(GitDiffSide::New),
        other => Err(CliError::Usage(format!(
            "--side must be 'old' or 'new' (got {other:?})"
        ))),
    }
}

fn parse_review_decision(decision: &str) -> Result<GitPrReviewDecision, CliError> {
    match decision {
        "approve" => Ok(GitPrReviewDecision::Approve),
        "request-changes" => Ok(GitPrReviewDecision::RequestChanges),
        other => Err(CliError::Usage(format!(
            "--decision must be 'approve' or 'request-changes' (got {other:?})"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_review_pr(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    pr: &str,
    decision: &str,
    commit: &str,
    body: Option<&str>,
    body_file: Option<&str>,
    to: &[String],
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    validate_hex64(pr)?;
    for recipient in to {
        validate_hex64(recipient)?;
    }
    // An empty body is allowed: the SDK substitutes the decision summary.
    let content = read_optional_body(body, body_file)?;

    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };
    let meta = GitPrCommentMeta {
        recipients: to.to_vec(),
        anchor: None,
        commit: Some(commit.to_string()),
        decision: Some(parse_review_decision(decision)?),
    };

    let builder = buzz_sdk::build_git_pr_comment(&repo, pr, &content, &meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_update_pr(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    pr: &str,
    pr_author: &str,
    commit: &str,
    clone_urls: &[String],
    body: Option<&str>,
    body_file: Option<&str>,
    merge_base: Option<&str>,
    euc: Option<&str>,
    to: &[String],
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    validate_hex64(pr)?;
    validate_hex64(pr_author)?;
    let content = read_optional_body(body, body_file)?;

    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };
    let meta = GitPrUpdateMeta {
        euc: euc.map(str::to_string),
        recipients: to.to_vec(),
        pr_event: pr.to_string(),
        pr_author: pr_author.to_string(),
        commit: commit.to_string(),
        clone_urls: clone_urls.to_vec(),
        merge_base: merge_base.map(str::to_string),
    };

    let builder = buzz_sdk::build_git_pr_update(&repo, &content, &meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_get_pr(client: &BuzzClient, event: &str) -> Result<(), CliError> {
    validate_hex64(event)?;
    let filter = serde_json::json!({
        "kinds": [1618],
        "ids": [event]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_list_prs(
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
        "kinds": [1618],
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
pub async fn cmd_pr_status(
    client: &BuzzClient,
    pr: &str,
    status: &str,
    body: Option<&str>,
    body_file: Option<&str>,
    repo_owner: Option<&str>,
    repo_id: Option<&str>,
    euc: Option<&str>,
    to: &[String],
    merge_commit: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(pr)?;
    let status = crate::commands::patches::parse_status(status)?;
    let content = read_optional_body(body, body_file)?;

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

    // Mirrors patch/issue status: default a `p` tag to the repo owner when
    // known; callers can add PR author/reviewers with repeated `--to`.
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
        root_event: pr.to_string(),
        accepted_revision_root: None,
        repo,
        euc: euc.map(str::to_string),
        recipients,
        applied_patches: vec![],
        merge_commit: merge_commit.map(str::to_string),
        applied_as_commits: vec![],
    };

    let builder = buzz_sdk::build_git_status(status, &content, &meta).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn dispatch(cmd: crate::PrCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::PrCmd;
    match cmd {
        PrCmd::Comment {
            repo_owner,
            repo_id,
            pr,
            body,
            body_file,
            file,
            line,
            side,
            commit,
            to,
        } => {
            cmd_comment_pr(
                client,
                &repo_owner,
                &repo_id,
                &pr,
                body.as_deref(),
                body_file.as_deref(),
                file.as_deref(),
                line,
                side.as_deref(),
                commit.as_deref(),
                &to,
            )
            .await
        }
        PrCmd::Review {
            repo_owner,
            repo_id,
            pr,
            decision,
            commit,
            body,
            body_file,
            to,
        } => {
            cmd_review_pr(
                client,
                &repo_owner,
                &repo_id,
                &pr,
                &decision,
                &commit,
                body.as_deref(),
                body_file.as_deref(),
                &to,
            )
            .await
        }
        PrCmd::Open {
            repo_owner,
            repo_id,
            subject,
            body,
            body_file,
            commit,
            clone,
            branch_name,
            merge_base,
            euc,
            label,
            to,
            channel,
            revision_of,
        } => {
            cmd_open_pr(
                client,
                &repo_owner,
                &repo_id,
                &subject,
                body.as_deref(),
                body_file.as_deref(),
                &commit,
                &clone,
                branch_name.as_deref(),
                merge_base.as_deref(),
                euc.as_deref(),
                &label,
                &to,
                channel.as_deref(),
                revision_of.as_deref(),
            )
            .await
        }
        PrCmd::Update {
            repo_owner,
            repo_id,
            pr,
            pr_author,
            commit,
            clone,
            body,
            body_file,
            merge_base,
            euc,
            to,
        } => {
            cmd_update_pr(
                client,
                &repo_owner,
                &repo_id,
                &pr,
                &pr_author,
                &commit,
                &clone,
                body.as_deref(),
                body_file.as_deref(),
                merge_base.as_deref(),
                euc.as_deref(),
                &to,
            )
            .await
        }
        PrCmd::Get { event } => cmd_get_pr(client, &event).await,
        PrCmd::List {
            repo_owner,
            repo_id,
            author,
            label,
            limit,
        } => {
            cmd_list_prs(
                client,
                &repo_owner,
                &repo_id,
                author.as_deref(),
                label.as_deref(),
                limit,
            )
            .await
        }
        PrCmd::Status {
            pr,
            status,
            body,
            body_file,
            repo_owner,
            repo_id,
            euc,
            to,
            merge_commit,
        } => {
            cmd_pr_status(
                client,
                &pr,
                &status,
                body.as_deref(),
                body_file.as_deref(),
                repo_owner.as_deref(),
                repo_id.as_deref(),
                euc.as_deref(),
                &to,
                merge_commit.as_deref(),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_optional_body_rejects_body_and_body_file_together() {
        assert!(read_optional_body(Some("body"), Some("file.md")).is_err());
    }

    #[test]
    fn read_optional_body_defaults_empty() {
        assert_eq!(read_optional_body(None, None).unwrap(), "");
    }
}
