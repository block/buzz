use std::collections::{HashMap, HashSet};

use crate::client::BuzzClient;
use crate::commands::with_git_provenance;
use crate::error::CliError;
use crate::validate::{read_or_stdin, sdk_err, validate_hex64, validate_repo_id};
use buzz_core::kind::{KIND_GIT_ISSUE, KIND_GIT_ISSUE_ASSIGNEE};
use buzz_sdk::{build_git_issue_assignment, GitIssueMeta, GitRepoCoord, GitStatusMeta};
use nostr::{Event, Timestamp};

const MAX_ASSIGNMENT_LIST_HEADS: u32 = 10_000;
const ISSUE_ROOT_IDS_PER_QUERY: usize = 500;

fn parse_events(json: &str) -> Result<Vec<Event>, CliError> {
    serde_json::from_str(json)
        .map_err(|error| CliError::Other(format!("failed to parse relay response: {error}")))
}

fn parse_event_values(values: Vec<serde_json::Value>) -> Result<Vec<Event>, CliError> {
    values
        .into_iter()
        .map(|value| {
            serde_json::from_value(value).map_err(|error| {
                CliError::Other(format!("failed to parse relay response event: {error}"))
            })
        })
        .collect()
}

fn monotonic_assignment_created_at(now: u64, prior_head: Option<u64>) -> u64 {
    prior_head.map_or(now, |prior| now.max(prior.saturating_add(1)))
}

fn current_assignment_filter(
    authors: &[String],
    issue: &str,
    repo_address: &str,
) -> serde_json::Value {
    serde_json::json!({
        "kinds": [KIND_GIT_ISSUE_ASSIGNEE],
        "authors": authors,
        "#d": [issue.to_ascii_lowercase()],
        "#a": [repo_address],
        "limit": authors.len(),
    })
}

fn assignment_authors_for_issue(
    root: &Event,
    repo_address: &str,
    repo_owner: &str,
) -> Result<Vec<String>, CliError> {
    if root.verify().is_err() {
        return Err(CliError::Other(
            "issue assignment target has an invalid signature".into(),
        ));
    }
    if u32::from(root.kind.as_u16()) != KIND_GIT_ISSUE {
        return Err(CliError::Other(
            "issue assignment target is not a kind:1621 issue".into(),
        ));
    }

    let root_a_tags: Vec<_> = root
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("a"))
        .collect();
    if root_a_tags.len() != 1
        || root_a_tags[0].as_slice().get(1).map(String::as_str) != Some(repo_address)
    {
        return Err(CliError::Other(
            "issue assignment repository does not match its issue root".into(),
        ));
    }

    let issue_author = root.pubkey.to_hex();
    let repo_owner = repo_owner.to_ascii_lowercase();
    if issue_author == repo_owner {
        Ok(vec![issue_author])
    } else {
        Ok(vec![issue_author, repo_owner])
    }
}

async fn fetch_assignment_authors(
    client: &BuzzClient,
    issue: &str,
    repo_address: &str,
    repo_owner: &str,
) -> Result<Vec<String>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_GIT_ISSUE],
        "ids": [issue],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let root = parse_events(&raw)?
        .into_iter()
        .find(|event| event.id.to_hex() == issue)
        .ok_or_else(|| CliError::NotFound(format!("issue {issue} was not found")))?;
    assignment_authors_for_issue(&root, repo_address, repo_owner)
}

fn assignment_issue_id<'a>(event: &'a Event, repo_address: &str) -> Option<&'a str> {
    if u32::from(event.kind.as_u16()) != KIND_GIT_ISSUE_ASSIGNEE
        || !event.content.is_empty()
        || event
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
    {
        return None;
    }

    let d_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("d"))
        .collect();
    let e_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("e"))
        .collect();
    let a_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("a"))
        .collect();
    if d_tags.len() != 1 || e_tags.len() != 1 || a_tags.len() != 1 {
        return None;
    }
    let d = d_tags[0].as_slice();
    let e = e_tags[0].as_slice();
    let a = a_tags[0].as_slice();
    if d.len() != 2
        || d[1].len() != 64
        || !d[1].chars().all(|character| character.is_ascii_hexdigit())
        || d[1] != d[1].to_ascii_lowercase()
        || e.len() != 4
        || e[1] != d[1]
        || !e[2].is_empty()
        || e[3] != "root"
        || a != ["a", repo_address]
    {
        return None;
    }

    let p_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("p"))
        .collect();
    let assignee_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("assignee"))
        .collect();
    let is_assignment = matches!(p_tags.as_slice(), [p]
        if p.as_slice().len() == 4
            && p.as_slice()[2].is_empty()
            && p.as_slice()[3] == "assignee"
            && p.as_slice()[1].len() == 64
            && p.as_slice()[1].chars().all(|character| character.is_ascii_hexdigit())
            && p.as_slice()[1] == p.as_slice()[1].to_ascii_lowercase())
        && assignee_tags.is_empty();
    let is_unassignment = p_tags.is_empty()
        && matches!(assignee_tags.as_slice(), [tag] if tag.as_slice() == ["assignee", "none"]);
    (is_assignment || is_unassignment).then_some(d[1].as_str())
}

fn assignment_assignee(event: &Event) -> Option<&str> {
    event.tags.iter().find_map(|tag| {
        let tag = tag.as_slice();
        (tag.len() == 4 && tag[0] == "p" && tag[2].is_empty() && tag[3] == "assignee")
            .then_some(tag[1].as_str())
    })
}

fn assignment_precedes(left: &Event, right: &Event) -> bool {
    left.created_at.as_secs() > right.created_at.as_secs()
        || (left.created_at == right.created_at && left.id < right.id)
}

fn latest_assignment_head(
    events: Vec<Event>,
    authors: &[String],
    issue: &str,
    repo_address: &str,
) -> Option<Event> {
    let mut events: Vec<_> = events
        .into_iter()
        .filter(|event| {
            event.verify().is_ok()
                && authors.contains(&event.pubkey.to_hex())
                && assignment_issue_id(event, repo_address) == Some(issue)
        })
        .collect();
    events.sort_by(|left, right| {
        right
            .created_at
            .as_secs()
            .cmp(&left.created_at.as_secs())
            .then_with(|| left.id.to_hex().cmp(&right.id.to_hex()))
    });
    events.into_iter().next()
}

fn resolve_current_assignments(
    assignment_events: Vec<Event>,
    issue_roots: &[Event],
    repo_address: &str,
    repo_owner: &str,
) -> Vec<Event> {
    let roots_by_id: HashMap<_, _> = issue_roots
        .iter()
        .filter(|root| root.verify().is_ok())
        .map(|root| (root.id.to_hex(), root))
        .collect();
    let mut current_by_issue: HashMap<String, Event> = HashMap::new();

    for event in assignment_events {
        if event.verify().is_err() {
            continue;
        }
        let Some(issue_id) = assignment_issue_id(&event, repo_address).map(str::to_string) else {
            continue;
        };
        let Some(root) = roots_by_id.get(&issue_id) else {
            continue;
        };
        let Ok(authors) = assignment_authors_for_issue(root, repo_address, repo_owner) else {
            continue;
        };
        if !authors.contains(&event.pubkey.to_hex()) {
            continue;
        }

        match current_by_issue.get(&issue_id) {
            Some(current) if !assignment_precedes(&event, current) => {}
            _ => {
                current_by_issue.insert(issue_id, event);
            }
        }
    }

    let mut current: Vec<_> = current_by_issue.into_values().collect();
    current.sort_by(|left, right| {
        right
            .created_at
            .as_secs()
            .cmp(&left.created_at.as_secs())
            .then_with(|| left.id.cmp(&right.id))
    });
    current
}

async fn fetch_issue_roots_for_assignments(
    client: &BuzzClient,
    issue_ids: &[String],
) -> Result<Vec<Event>, CliError> {
    let mut roots = Vec::new();
    for ids in issue_ids.chunks(ISSUE_ROOT_IDS_PER_QUERY) {
        let filter = serde_json::json!({
            "kinds": [KIND_GIT_ISSUE],
            "ids": ids,
        });
        roots.extend(parse_event_values(
            client.query_paginated(filter, ids.len() as u32).await?,
        )?);
    }
    Ok(roots)
}

async fn fetch_current_issue_assignment(
    client: &BuzzClient,
    authors: &[String],
    issue: &str,
    repo_address: &str,
) -> Result<Option<Event>, CliError> {
    let filter = current_assignment_filter(authors, issue, repo_address);
    let raw = client.query(&filter).await?;
    Ok(latest_assignment_head(
        parse_events(&raw)?,
        authors,
        issue,
        repo_address,
    ))
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
    if limit == Some(0) {
        println!("[]");
        return Ok(());
    }

    let repo_owner = repo_owner.to_ascii_lowercase();
    let a_value = format!("30617:{repo_owner}:{repo_id}");
    let filter = serde_json::json!({
        "kinds": [KIND_GIT_ISSUE_ASSIGNEE],
        "#a": [a_value.clone()]
    });
    let assignment_values = client
        .query_paginated(filter, MAX_ASSIGNMENT_LIST_HEADS + 1)
        .await?;
    if assignment_values.len() > MAX_ASSIGNMENT_LIST_HEADS as usize {
        return Err(CliError::Other(format!(
            "assignment listing exceeds the safety limit of {MAX_ASSIGNMENT_LIST_HEADS} current heads"
        )));
    }
    let assignment_events = parse_event_values(assignment_values)?;
    let issue_ids: Vec<String> = assignment_events
        .iter()
        .filter_map(|event| assignment_issue_id(event, &a_value))
        .map(str::to_string)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if issue_ids.is_empty() {
        println!("[]");
        return Ok(());
    }

    let issue_roots = fetch_issue_roots_for_assignments(client, &issue_ids).await?;
    let mut current =
        resolve_current_assignments(assignment_events, &issue_roots, &a_value, &repo_owner);
    if let Some(pubkey) = assignee {
        let pubkey = pubkey.to_ascii_lowercase();
        current.retain(|event| assignment_assignee(event) == Some(pubkey.as_str()));
    }
    if let Some(limit) = limit {
        current.truncate(limit as usize);
    }

    println!(
        "{}",
        serde_json::to_string(&current).map_err(|error| CliError::Other(format!(
            "failed to serialize assignments: {error}"
        )))?
    );
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
        owner: repo_owner.to_ascii_lowercase(),
        id: repo_id.to_string(),
    };
    let repo_address = format!("30617:{}:{}", repo.owner, repo.id);
    let authors = fetch_assignment_authors(client, &issue, &repo_address, &repo.owner).await?;
    let signer = client.keys().public_key().to_hex();
    if !authors.contains(&signer) {
        return Err(CliError::Auth(
            "only the issue author or repository owner can assign or unassign this issue".into(),
        ));
    }

    let prior = fetch_current_issue_assignment(client, &authors, &issue, &repo_address).await?;
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
    let disposition = assignment_write_disposition(&resp)?;
    // NIP-33 replacement is author-scoped, so two authorized writers can both
    // receive "saved" while only one wins the cross-author projection. Verify
    // every accepted write, not only same-author duplicate responses.
    let current = fetch_current_issue_assignment(client, &authors, &issue, &repo_address)
        .await
        .map_err(|error| {
            CliError::DeliveryUnknown(format!(
                "assignment was accepted, but its current head could not be verified: {error}"
            ))
        })?;
    if current.as_ref().map(|event| event.id.to_hex()).as_deref() != Some(submitted_id.as_str()) {
        return Err(CliError::Conflict(
            "assignment changed concurrently; fetch the current assignment and retry".into(),
        ));
    }

    match disposition {
        AssignmentWriteDisposition::Applied => println!("{resp}"),
        AssignmentWriteDisposition::Duplicate => {
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
        assignment_authors_for_issue, assignment_write_disposition, current_assignment_filter,
        latest_assignment_head, monotonic_assignment_created_at, resolve_current_assignments,
        AssignmentWriteDisposition,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn issue_event(author: &Keys, repo_address: &str) -> nostr::Event {
        EventBuilder::new(Kind::Custom(1621), "body")
            .tags([Tag::parse(["a", repo_address]).unwrap()])
            .sign_with_keys(author)
            .unwrap()
    }

    fn assignment_event(author: &Keys, issue: &str, repo_address: &str, at: u64) -> nostr::Event {
        EventBuilder::new(Kind::Custom(32001), "")
            .tags([
                Tag::parse(["d", issue]).unwrap(),
                Tag::parse(["e", issue, "", "root"]).unwrap(),
                Tag::parse(["assignee", "none"]).unwrap(),
                Tag::parse(["a", repo_address]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(author)
            .unwrap()
    }

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
    fn assignment_head_filter_queries_every_author_scoped_head() {
        let authors = vec!["a".repeat(64), "b".repeat(64)];
        let repo = format!("30617:{}:demo", authors[1]);
        let filter = current_assignment_filter(&authors, &"E".repeat(64), &repo);
        assert_eq!(filter["authors"], serde_json::json!(authors));
        assert_eq!(filter["#d"], serde_json::json!(["e".repeat(64)]));
        assert_eq!(filter["#a"], serde_json::json!([repo]));
        assert_eq!(filter["limit"], 2);
    }

    #[test]
    fn assignment_authority_comes_from_the_signed_issue_and_repo_coordinate() {
        let owner = Keys::generate();
        let reporter = Keys::generate();
        let repo = format!("30617:{}:demo", owner.public_key().to_hex());
        let issue = issue_event(&reporter, &repo);

        assert_eq!(
            assignment_authors_for_issue(&issue, &repo, &owner.public_key().to_hex()).unwrap(),
            vec![reporter.public_key().to_hex(), owner.public_key().to_hex()]
        );

        let wrong_repo = format!("30617:{}:other", owner.public_key().to_hex());
        assert!(
            assignment_authors_for_issue(&issue, &wrong_repo, &owner.public_key().to_hex())
                .is_err()
        );

        let mut tampered = serde_json::to_value(&issue).unwrap();
        tampered["content"] = serde_json::json!("tampered");
        let tampered: nostr::Event = serde_json::from_value(tampered).unwrap();
        assert!(
            assignment_authors_for_issue(&tampered, &repo, &owner.public_key().to_hex())
                .unwrap_err()
                .to_string()
                .contains("invalid signature")
        );
    }

    #[test]
    fn repo_owner_issue_author_is_not_queried_twice() {
        let owner = Keys::generate();
        let repo = format!("30617:{}:demo", owner.public_key().to_hex());
        let issue = issue_event(&owner, &repo);

        assert_eq!(
            assignment_authors_for_issue(&issue, &repo, &owner.public_key().to_hex()).unwrap(),
            vec![owner.public_key().to_hex()]
        );
    }

    #[test]
    fn latest_assignment_resolves_across_author_heads_and_ties_deterministically() {
        let owner = Keys::generate();
        let reporter = Keys::generate();
        let repo = format!("30617:{}:demo", owner.public_key().to_hex());
        let issue = "e".repeat(64);
        let older_owner = assignment_event(&owner, &issue, &repo, 100);
        let newer_reporter = assignment_event(&reporter, &issue, &repo, 101);

        assert_eq!(
            latest_assignment_head(
                vec![newer_reporter.clone(), older_owner],
                &[owner.public_key().to_hex(), reporter.public_key().to_hex()],
                &issue,
                &repo,
            )
            .unwrap()
            .id,
            newer_reporter.id
        );

        let tied_owner = assignment_event(&owner, &issue, &repo, 200);
        let tied_reporter = assignment_event(&reporter, &issue, &repo, 200);
        let expected = if tied_owner.id < tied_reporter.id {
            tied_owner.id
        } else {
            tied_reporter.id
        };
        assert_eq!(
            latest_assignment_head(
                vec![tied_owner, tied_reporter],
                &[owner.public_key().to_hex(), reporter.public_key().to_hex()],
                &issue,
                &repo,
            )
            .unwrap()
            .id,
            expected
        );
    }

    #[test]
    fn assignment_listing_drops_unauthorized_and_stale_author_heads() {
        let owner = Keys::generate();
        let reporter = Keys::generate();
        let attacker = Keys::generate();
        let repo = format!("30617:{}:demo", owner.public_key().to_hex());
        let root = issue_event(&reporter, &repo);
        let issue = root.id.to_hex();
        let stale_owner = assignment_event(&owner, &issue, &repo, 100);
        let current_reporter = assignment_event(&reporter, &issue, &repo, 200);
        let unauthorized = assignment_event(&attacker, &issue, &repo, 300);

        let current = resolve_current_assignments(
            vec![unauthorized, stale_owner, current_reporter.clone()],
            std::slice::from_ref(&root),
            &repo,
            &owner.public_key().to_hex(),
        );
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].id, current_reporter.id);

        let mut tampered = serde_json::to_value(&current[0]).unwrap();
        tampered["created_at"] = serde_json::json!(300);
        let tampered: nostr::Event = serde_json::from_value(tampered).unwrap();
        assert!(resolve_current_assignments(
            vec![tampered],
            std::slice::from_ref(&root),
            &repo,
            &owner.public_key().to_hex(),
        )
        .is_empty());
    }
}
