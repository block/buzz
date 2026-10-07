use std::collections::{HashMap, HashSet};

use crate::client::BuzzClient;
use crate::commands::with_git_provenance;
use crate::commands::GIT_ORIGIN_CHANNEL_ENV;
use crate::error::CliError;
use crate::validate::{read_or_stdin, sdk_err, validate_hex64, validate_repo_id};
use buzz_sdk::{GitIssueMeta, GitRepoCoord, GitStatusMeta};
use nostr::Timestamp;
use serde::Deserialize;

const ISSUE_ASSIGNMENT_LABEL: &str = "assignment";
const ISSUE_UNASSIGNMENT_LABEL: &str = "unassignment";

fn assignment_note_label(assignees: &[String], label: Option<&str>) -> Result<String, CliError> {
    if let Some(label) = label {
        let label = label.trim();
        if label.is_empty() || label.chars().count() > 128 {
            return Err(CliError::Usage(
                "--label must be between 1 and 128 characters".into(),
            ));
        }
        return Ok(label.to_string());
    }
    let prefixes = assignees
        .iter()
        .map(|assignee| format!("{}…", assignee.chars().take(8).collect::<String>()))
        .collect::<Vec<_>>();
    for included in (0..=prefixes.len()).rev() {
        let omitted = prefixes.len() - included;
        let mut generated = prefixes[..included].join(", ");
        if omitted > 0 {
            let suffix = format!("{omitted} other{}", if omitted == 1 { "" } else { "s" });
            if !generated.is_empty() {
                generated.push_str(", and ");
            }
            generated.push_str(&suffix);
        }
        if !generated.is_empty() && generated.chars().count() <= 128 {
            return Ok(generated);
        }
    }
    Err(CliError::Usage(
        "Unable to generate an assignee label between 1 and 128 characters".into(),
    ))
}

#[derive(Clone, Copy)]
enum IssueAssignmentOperation {
    Assign,
    Unassign,
}

#[derive(Debug)]
struct AssignmentEvent {
    id: String,
    pubkey: String,
    created_at: u64,
    tags: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct AssignmentQueryEvent {
    id: String,
    kind: u16,
    pubkey: String,
    created_at: u64,
    tags: Vec<Vec<String>>,
}

impl From<&AssignmentQueryEvent> for AssignmentEvent {
    fn from(event: &AssignmentQueryEvent) -> Self {
        Self {
            id: event.id.clone(),
            pubkey: event.pubkey.clone(),
            created_at: event.created_at,
            tags: event.tags.clone(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AssignmentState {
    assignees: HashSet<String>,
    heads: HashMap<String, String>,
}

#[derive(Debug)]
struct ParsedAssignmentOperation {
    id: String,
    is_assignment: bool,
    pubkeys: Vec<String>,
    prior: Option<String>,
}

fn tag_values<'a>(event: &'a AssignmentEvent, name: &str) -> Vec<&'a str> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            if tag.first().map(String::as_str) != Some(name) {
                return None;
            }
            tag.get(1)
                .map(String::as_str)
                .filter(|value| !value.is_empty())
        })
        .collect()
}

fn has_root_tag(event: &AssignmentEvent, issue_id: &str) -> bool {
    event.tags.iter().any(|tag| {
        matches!(
            tag.as_slice(),
            [name, value, ..] if name == "e" && value == issue_id
        )
    })
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn apply_assignment_operation(state: &mut AssignmentState, operation: ParsedAssignmentOperation) {
    if let Some(prior) = operation.prior.as_ref() {
        let Some(target) = operation.pubkeys.first() else {
            return;
        };
        if state.heads.get(target) != Some(prior) {
            return;
        }
    }
    for pubkey in operation.pubkeys {
        if operation.is_assignment {
            state.assignees.insert(pubkey.clone());
        } else {
            state.assignees.remove(&pubkey);
        }
        state.heads.insert(pubkey, operation.id.clone());
    }
}

fn reduce_assignment_operations(
    issue_id: &str,
    issue_author: &str,
    repo_owner: &str,
    events: &[AssignmentEvent],
) -> AssignmentState {
    let issue_author = issue_author.to_ascii_lowercase();
    let repo_owner = repo_owner.to_ascii_lowercase();
    let mut events = events.iter().collect::<Vec<_>>();
    events.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut uncaused_self_operations = Vec::new();
    let mut authoritative_operations = Vec::new();
    let mut causal_self_operations = Vec::new();
    for event in events {
        if !has_root_tag(event, issue_id) {
            continue;
        }
        let labels = tag_values(event, "t");
        let is_assignment = labels.contains(&ISSUE_ASSIGNMENT_LABEL);
        let is_unassignment = labels.contains(&ISSUE_UNASSIGNMENT_LABEL);
        if is_assignment == is_unassignment {
            continue;
        }
        let signer = event.pubkey.to_ascii_lowercase();
        let pubkeys = tag_values(event, "p")
            .into_iter()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        let is_authoritative = signer == issue_author || signer == repo_owner;
        let is_self_operation = pubkeys.len() == 1 && pubkeys[0] == signer;
        if !is_authoritative && !is_self_operation {
            continue;
        }

        let mut operation = ParsedAssignmentOperation {
            id: event.id.to_ascii_lowercase(),
            is_assignment,
            pubkeys,
            prior: None,
        };
        if is_authoritative {
            authoritative_operations.push(operation);
            continue;
        }

        let prior_tags = event
            .tags
            .iter()
            .filter(|tag| tag.first().map(String::as_str) == Some("prior"))
            .collect::<Vec<_>>();
        if prior_tags.is_empty() {
            uncaused_self_operations.push(operation);
        } else if prior_tags.len() == 1 && prior_tags[0].get(1).is_some_and(|prior| is_hex64(prior))
        {
            operation.prior = prior_tags[0].get(1).map(|prior| prior.to_ascii_lowercase());
            causal_self_operations.push(operation);
        }
    }

    let mut state = AssignmentState::default();
    for operation in uncaused_self_operations
        .into_iter()
        .chain(authoritative_operations)
        .chain(causal_self_operations)
    {
        apply_assignment_operation(&mut state, operation);
    }
    state
}

struct IssueAssignmentContext {
    created_at: u64,
    prior: Option<String>,
}

impl IssueAssignmentOperation {
    fn content(self, label: &str) -> String {
        match self {
            Self::Assign => format!("Assigned this issue to {label}"),
            Self::Unassign => format!("Unassigned {label} from this issue"),
        }
    }
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

async fn resolve_issue_repo_target(
    client: &BuzzClient,
    repo_owner: Option<&str>,
    repo_id: Option<&str>,
    channel: Option<&str>,
) -> Result<(String, String), CliError> {
    let owner = repo_owner.map(str::trim).filter(|value| !value.is_empty());
    let id = repo_id.map(str::trim).filter(|value| !value.is_empty());
    match (owner, id) {
        (Some(owner), Some(id)) => Ok((owner.to_string(), id.to_string())),
        (None, None) => {
            let channel = channel
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| std::env::var(GIT_ORIGIN_CHANNEL_ENV).ok());
            let Some(channel) = channel else {
                return Err(CliError::Usage(
                    "provide --repo-owner and --repo-id, or --channel (or set BUZZ_GIT_ORIGIN_CHANNEL_ID)".into(),
                ));
            };
            let resolved = crate::commands::project_channel::resolve_or_ensure_repo_for_channel(
                client, &channel,
            )
            .await?;
            Ok((resolved.repo_owner, resolved.repo_id))
        }
        _ => Err(CliError::Usage(
            "provide both --repo-owner and --repo-id, or --channel".into(),
        )),
    }
}

/// Publish an issue assignment: a kind:1 comment on the issue whose `p`
/// tags are the assignees, labeled `t: assignment` (same event shape the
/// Desktop app writes). Clients trust it when signed by the issue author
/// or repo owner, or when it is a self-assignment.
pub async fn cmd_assign_issue(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    assignees: &[String],
    label: Option<&str>,
) -> Result<(), CliError> {
    publish_issue_assignment_operation(
        client,
        issue,
        repo_owner,
        repo_id,
        assignees,
        label,
        IssueAssignmentOperation::Assign,
    )
    .await
}

/// Publish an issue unassignment with the same trust rules as assignment.
pub async fn cmd_unassign_issue(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    assignees: &[String],
    label: Option<&str>,
) -> Result<(), CliError> {
    publish_issue_assignment_operation(
        client,
        issue,
        repo_owner,
        repo_id,
        assignees,
        label,
        IssueAssignmentOperation::Unassign,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn publish_issue_assignment_operation(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    assignees: &[String],
    label: Option<&str>,
    operation: IssueAssignmentOperation,
) -> Result<(), CliError> {
    validate_hex64(issue)?;
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    for assignee in assignees {
        validate_hex64(assignee)?;
    }

    let label = assignment_note_label(assignees, label)?;
    let content = operation.content(&label);
    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };
    let signer = client.keys().public_key().to_hex();
    let is_self_service = assignees.len() == 1
        && assignees[0].eq_ignore_ascii_case(&signer)
        && !signer.eq_ignore_ascii_case(repo_owner);
    let context = issue_assignment_context(client, issue, &repo, &signer, is_self_service).await?;
    let builder = match (operation, is_self_service) {
        (IssueAssignmentOperation::Assign, true) => {
            buzz_sdk::build_git_issue_assignment_with_prior(
                &repo,
                issue,
                assignees,
                &content,
                context.prior.as_deref(),
            )
        }
        (IssueAssignmentOperation::Unassign, true) => {
            buzz_sdk::build_git_issue_unassignment_with_prior(
                &repo,
                issue,
                assignees,
                &content,
                context.prior.as_deref(),
            )
        }
        (IssueAssignmentOperation::Assign, false) => {
            buzz_sdk::build_git_issue_assignment(&repo, issue, assignees, &content)
        }
        (IssueAssignmentOperation::Unassign, false) => {
            buzz_sdk::build_git_issue_unassignment(&repo, issue, assignees, &content)
        }
    }
    .map(|builder| builder.custom_created_at(Timestamp::from_secs(context.created_at)));
    let event = client.sign_event(builder.map_err(sdk_err)?)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

async fn issue_assignment_context(
    client: &BuzzClient,
    issue: &str,
    repo: &GitRepoCoord,
    signer: &str,
    include_prior: bool,
) -> Result<IssueAssignmentContext, CliError> {
    let root_filter = serde_json::json!({
        "kinds": [1621],
        "ids": [issue],
        "limit": 1
    });
    let assignment_filter = serde_json::json!({
        "kinds": [1],
        "#e": [issue],
        "#t": [ISSUE_ASSIGNMENT_LABEL, ISSUE_UNASSIGNMENT_LABEL],
        "limit": 500
    });
    let signer_comment_filter = serde_json::json!({
        "kinds": [1],
        "#e": [issue],
        "authors": [signer],
        "limit": 1
    });
    let response = client
        .query_multi(&[root_filter, assignment_filter, signer_comment_filter])
        .await?;
    // CLI read responses intentionally omit signatures, so deserialize only
    // the event fields needed for assignment reduction.
    let events = serde_json::from_str::<Vec<AssignmentQueryEvent>>(&response)
        .map_err(|error| CliError::Other(format!("parse issue assignment context: {error}")))?;
    let root = events
        .iter()
        .find(|event| event.kind == 1621 && event.id == issue)
        .ok_or_else(|| CliError::Other("issue root was not returned by the relay".into()))?;
    let expected_repo = format!("30617:{}:{}", repo.owner.to_ascii_lowercase(), repo.id);
    let root_matches_repo = root.tags.iter().any(|tag| {
        tag.as_slice().first().map(String::as_str) == Some("a")
            && tag.as_slice().get(1) == Some(&expected_repo)
    });
    if !root_matches_repo {
        return Err(CliError::Other(
            "issue root does not match the requested repository".into(),
        ));
    }

    let comments = events
        .iter()
        .filter(|event| event.kind == 1)
        .map(AssignmentEvent::from)
        .collect::<Vec<_>>();
    let latest = comments
        .iter()
        .filter(|event| event.pubkey.eq_ignore_ascii_case(signer))
        .map(|event| event.created_at)
        .max()
        .unwrap_or(0);
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CliError::Other(format!("read system clock: {error}")))?
        .as_secs()
        .max(latest.saturating_add(1));
    let prior = include_prior
        .then(|| {
            reduce_assignment_operations(issue, &root.pubkey, &repo.owner, &comments)
                .heads
                .get(&signer.to_ascii_lowercase())
                .cloned()
        })
        .flatten();
    Ok(IssueAssignmentContext { created_at, prior })
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
            channel,
            title,
            content,
            label,
            to,
        } => {
            let (repo_owner, repo_id) = resolve_issue_repo_target(
                client,
                repo_owner.as_deref(),
                repo_id.as_deref(),
                channel.as_deref(),
            )
            .await?;
            cmd_create_issue(client, &repo_owner, &repo_id, &title, &content, &label, &to).await
        }
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
        IssuesCmd::Assign {
            issue,
            repo_owner,
            repo_id,
            assignee,
            label,
        } => {
            cmd_assign_issue(
                client,
                &issue,
                &repo_owner,
                &repo_id,
                &assignee,
                label.as_deref(),
            )
            .await
        }
        IssuesCmd::Unassign {
            issue,
            repo_owner,
            repo_id,
            assignee,
            label,
        } => {
            cmd_unassign_issue(
                client,
                &issue,
                &repo_owner,
                &repo_id,
                &assignee,
                label.as_deref(),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assignment_note_label, reduce_assignment_operations, AssignmentEvent, AssignmentQueryEvent,
        ISSUE_ASSIGNMENT_LABEL, ISSUE_UNASSIGNMENT_LABEL,
    };

    const ISSUE: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    const AUTHOR: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const VOLUNTEER: &str = "5555555555555555555555555555555555555555555555555555555555555555";

    fn assignment_event(
        pubkey: &str,
        id: &str,
        assignment: bool,
        created_at: u64,
        prior: Option<&str>,
    ) -> AssignmentEvent {
        let mut tags = vec![
            vec!["e".into(), ISSUE.into(), "".into(), "root".into()],
            vec!["p".into(), VOLUNTEER.into()],
            vec![
                "t".into(),
                if assignment {
                    ISSUE_ASSIGNMENT_LABEL.into()
                } else {
                    ISSUE_UNASSIGNMENT_LABEL.into()
                },
            ],
        ];
        if let Some(prior) = prior {
            tags.push(vec!["prior".into(), prior.into()]);
        }
        AssignmentEvent {
            id: id.into(),
            pubkey: pubkey.into(),
            created_at,
            tags,
        }
    }

    #[test]
    fn assignment_note_label_enforces_desktop_length_limit() {
        let assignees = vec!["a".repeat(64)];
        assert_eq!(
            assignment_note_label(&assignees, Some(" Thomas ")).unwrap(),
            "Thomas"
        );
        assert!(assignment_note_label(&assignees, Some("")).is_err());
        assert!(assignment_note_label(&assignees, Some(&"x".repeat(129))).is_err());
        assert_eq!(
            assignment_note_label(&assignees, None).unwrap(),
            "aaaaaaaa…"
        );
        let many_assignees = (0..50)
            .map(|index| format!("{index:064x}"))
            .collect::<Vec<_>>();
        let generated = assignment_note_label(&many_assignees, None).unwrap();
        assert!(generated.chars().count() <= 128);
        assert!(generated.contains("others"));
    }

    #[test]
    fn assignment_query_event_accepts_sig_stripped_cli_reads() {
        let event = serde_json::from_value::<AssignmentQueryEvent>(serde_json::json!({
            "id": "1".repeat(64),
            "kind": 1,
            "pubkey": VOLUNTEER,
            "created_at": 200,
            "tags": [["e", ISSUE, "", "root"]]
        }))
        .unwrap();

        assert_eq!(event.kind, 1);
        assert_eq!(event.pubkey, VOLUNTEER);
    }

    #[test]
    fn uncaused_future_self_operations_lose_to_authority() {
        let owner_unassign = "1".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(VOLUNTEER, &"2".repeat(64), true, 1_000, None),
                assignment_event(OWNER, &owner_unassign, false, 200, None),
            ],
        );
        assert!(!state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&owner_unassign));

        let owner_assign = "3".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(VOLUNTEER, &"4".repeat(64), false, 1_000, None),
                assignment_event(OWNER, &owner_assign, true, 200, None),
            ],
        );
        assert!(state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&owner_assign));
    }

    #[test]
    fn causal_self_operations_can_follow_authority() {
        let owner_assign = "5".repeat(64);
        let self_unassign = "6".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(OWNER, &owner_assign, true, 200, None),
                assignment_event(VOLUNTEER, &self_unassign, false, 300, Some(&owner_assign)),
            ],
        );
        assert!(!state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&self_unassign));

        let owner_unassign = "7".repeat(64);
        let self_assign = "8".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(OWNER, &owner_unassign, false, 200, None),
                assignment_event(VOLUNTEER, &self_assign, true, 300, Some(&owner_unassign)),
            ],
        );
        assert!(state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&self_assign));
    }

    #[test]
    fn stale_causal_self_operation_is_ignored() {
        let initial_assign = "9".repeat(64);
        let owner_unassign = "a".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(OWNER, &initial_assign, true, 100, None),
                assignment_event(OWNER, &owner_unassign, false, 200, None),
                assignment_event(VOLUNTEER, &"c".repeat(64), true, 300, Some(&initial_assign)),
            ],
        );

        assert!(!state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&owner_unassign));
    }
}
